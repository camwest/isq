//! GitHub API client implementation

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::stream::{self, StreamExt};
use once_cell::sync::Lazy;
use serde::Deserialize;
use tokio::sync::{Mutex, Semaphore};

use crate::forges::{CreateGoalRequest, FetchResult, Label};
use crate::repo::Repo;

use super::types::{GitHubComment, GitHubIssue, GitHubLabel, GitHubMilestone, SearchResult};

const PER_PAGE: usize = 100;

// GitHub secondary rate limits (from docs):
// - Max 100 concurrent requests
// - Max 900 points/min (GET=1pt, POST/PATCH/PUT/DELETE=5pts)
// - Wait at least 1 sec between write requests
const MAX_CONCURRENT_REQUESTS: usize = 80; // Stay safely under 100
const WRITE_SPACING: Duration = Duration::from_secs(1);
const MAX_RETRIES: u32 = 3;

// Global rate limiting state
static REQUEST_SEMAPHORE: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)));
static LAST_WRITE_TIME: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

/// Create HTTP client with appropriate settings
pub fn create_http_client() -> reqwest::Client {
    crate::forges::create_http_client()
}

/// Throttle write requests to maintain 1 sec spacing
async fn throttle_write() {
    let mut last = LAST_WRITE_TIME.lock().await;
    if let Some(last_time) = *last {
        let elapsed = last_time.elapsed();
        if elapsed < WRITE_SPACING {
            tokio::time::sleep(WRITE_SPACING - elapsed).await;
        }
    }
    *last = Some(Instant::now());
}

/// Check if response indicates rate limiting
fn is_rate_limited(status: u16, body: &str) -> bool {
    (status == 403 || status == 429)
        && (body.contains("rate limit") || body.contains("secondary rate limit"))
}

/// Parse the last page number from GitHub's Link header
/// Example: <https://api.github.com/...?page=5>; rel="last"
fn parse_last_page_from_link_header(link_header: &str) -> Option<usize> {
    for part in link_header.split(',') {
        if part.contains("rel=\"last\"") {
            // Extract URL between < and >
            if let Some(start) = part.find('<') {
                if let Some(end) = part.find('>') {
                    let url = &part[start + 1..end];
                    // Extract page parameter (must be preceded by ? or &, not part of per_page)
                    for prefix in ["?page=", "&page="] {
                        if let Some(page_start) = url.find(prefix) {
                            let page_str = &url[page_start + prefix.len()..];
                            // Take digits until non-digit
                            let page_num: String = page_str
                                .chars()
                                .take_while(|c| c.is_ascii_digit())
                                .collect();
                            return page_num.parse().ok();
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parse retry-after header or use exponential backoff
fn get_retry_delay(response: &reqwest::Response, attempt: u32) -> Duration {
    // Check retry-after header first
    if let Some(retry_after) = response.headers().get("retry-after") {
        if let Ok(secs) = retry_after.to_str().unwrap_or("").parse::<u64>() {
            return Duration::from_secs(secs);
        }
    }
    // Exponential backoff: 1s, 2s, 4s
    Duration::from_secs(1 << attempt)
}

#[derive(Clone)]
pub struct GitHubClient {
    client: reqwest::Client,
    token: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self {
            client: create_http_client(),
            token,
        }
    }

    /// Fetch all issues for a repo (parallel pagination with rate limiting)
    /// Returns FetchResult with is_complete=false if any pages failed
    pub async fn list_issues_internal(
        &self,
        repo: &Repo,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<crate::forges::Issue>> {
        // For incremental sync with since parameter, we can't use search API count
        // because it doesn't support since. Use sequential pagination instead.
        if since.is_some() {
            return self
                .list_issues_since_sequential(repo, since.unwrap())
                .await;
        }

        // Get total count from search API
        let total = self.get_issue_count(repo).await?;

        if total == 0 {
            return Ok(FetchResult::complete(Vec::new()));
        }

        let total_pages = (total + PER_PAGE - 1) / PER_PAGE;
        eprintln!("Fetching {} issues across {} pages...", total, total_pages);

        // Fetch all pages in parallel with semaphore-bounded concurrency
        let futures = (1..=total_pages).map(|page| {
            let client = self.clone();
            let repo = repo.clone();
            async move {
                // Acquire semaphore permit before making request
                let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();
                client.fetch_page_with_retry(&repo, page, None).await
            }
        });

        // Stream results as they complete, printing progress
        let mut stream = stream::iter(futures).buffer_unordered(MAX_CONCURRENT_REQUESTS);
        let mut all_issues = Vec::with_capacity(total);
        let mut error_count = 0;
        let mut rate_limit_errors = 0;
        let mut completed = 0;

        while let Some(result) = stream.next().await {
            match result {
                Ok(issues) => all_issues.extend(issues),
                Err(e) => {
                    let err_str = e.to_string();
                    eprintln!("Warning: page fetch failed: {}", err_str);
                    if err_str.contains("rate limit") || err_str.contains("403") {
                        rate_limit_errors += 1;
                    }
                    error_count += 1;
                }
            }
            completed += 1;
            if completed % 10 == 0 || completed == total_pages {
                eprintln!("  {}/{} pages", completed, total_pages);
            }
        }

        // If we expected issues but got none, sync failed completely
        if all_issues.is_empty() && total > 0 {
            let reason = if rate_limit_errors > 0 {
                "rate limit"
            } else {
                "network error"
            };
            anyhow::bail!(
                "Sync failed ({}): all {} page fetches failed (expected {} issues)",
                reason,
                total_pages,
                total
            );
        }

        // Warn if we got partial results
        let is_complete = error_count == 0;
        if error_count > 0 && !all_issues.is_empty() {
            eprintln!(
                "Warning: {} of {} pages failed, got {} of {} expected issues",
                error_count,
                total_pages,
                all_issues.len(),
                total
            );
        }

        Ok(FetchResult {
            items: all_issues,
            is_complete,
        })
    }

    /// Fetch issues updated since timestamp (sequential pagination for incremental sync)
    async fn list_issues_since_sequential(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<crate::forges::Issue>> {
        let mut all_issues = Vec::new();
        let mut page = 1;

        loop {
            let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();
            match self.fetch_page_with_retry(repo, page, Some(&since)).await {
                Ok(issues) => {
                    let count = issues.len();
                    all_issues.extend(issues);
                    if count < PER_PAGE {
                        break;
                    }
                    page += 1;
                }
                Err(e) => {
                    eprintln!("Warning: page {} fetch failed: {}", page, e);
                    // For incremental, bail on first error - we can retry the whole thing
                    return Ok(FetchResult::incomplete(all_issues));
                }
            }
        }

        Ok(FetchResult::complete(all_issues))
    }

    /// Get total issue count via search API
    async fn get_issue_count(&self, repo: &Repo) -> Result<usize> {
        let url = format!(
            "https://api.github.com/search/issues?q=repo:{}/{}+is:issue&per_page=1",
            repo.owner, repo.name
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub search API error {}: {}", status, body);
        }

        let result: SearchResult = response.json().await?;
        Ok(result.total_count)
    }

    /// Fetch a single page of issues with retry on rate limit or network errors
    async fn fetch_page_with_retry(
        &self,
        repo: &Repo,
        page: usize,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<crate::forges::Issue>> {
        let base_url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=all&per_page={}&page={}",
            repo.owner, repo.name, PER_PAGE, page
        );
        let url = match since {
            // Use Z suffix (not +00:00) to avoid URL encoding issues
            Some(ts) => format!(
                "{}&since={}",
                base_url,
                ts.to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            None => base_url,
        };

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            // Handle network/connection errors with retry
            let response = match self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "isq")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) if attempt < MAX_RETRIES - 1 => {
                    let delay = Duration::from_secs(1 << attempt);
                    eprintln!(
                        "Network error on page {}, retrying in {:?} (attempt {}/{}): {}",
                        page,
                        delay,
                        attempt + 1,
                        MAX_RETRIES,
                        e
                    );
                    last_error = Some(e.to_string());
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            if response.status().is_success() {
                // Handle JSON decode errors with retry
                match response.json::<Vec<GitHubIssue>>().await {
                    Ok(issues) => {
                        return Ok(issues
                            .into_iter()
                            .filter(|i| i.pull_request.is_none()) // Filter PRs at source
                            .map(|i| i.into_issue())
                            .collect());
                    }
                    Err(e) if attempt < MAX_RETRIES - 1 => {
                        let delay = Duration::from_secs(1 << attempt);
                        eprintln!(
                            "Decode error on page {}, retrying in {:?} (attempt {}/{}): {}",
                            page,
                            delay,
                            attempt + 1,
                            MAX_RETRIES,
                            e
                        );
                        last_error = Some(e.to_string());
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            let status = response.status().as_u16();
            let delay = get_retry_delay(&response, attempt);
            let body = response.text().await?;

            if is_rate_limited(status, &body) && attempt < MAX_RETRIES - 1 {
                eprintln!(
                    "Rate limited on page {}, retrying in {:?} (attempt {}/{})",
                    page,
                    delay,
                    attempt + 1,
                    MAX_RETRIES
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        anyhow::bail!(
            "Max retries exceeded for page {}: {}",
            page,
            last_error.unwrap_or_default()
        )
    }

    /// Get authenticated user's login
    pub async fn get_user(&self) -> Result<String> {
        let response = self
            .client
            .get("https://api.github.com/user")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        let user: User = response.json().await?;
        Ok(user.login)
    }

    /// Helper for PATCH requests to update issue state
    pub async fn patch_issue(
        &self,
        repo: &Repo,
        number: u64,
        body: &serde_json::Value,
    ) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}",
            repo.owner, repo.name, number
        );

        let response = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Fetch all comments for a repo (parallel pagination with rate limiting)
    /// Uses repo-level endpoint: GET /repos/{owner}/{repo}/issues/comments
    pub async fn list_all_comments_internal(
        &self,
        repo: &Repo,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<GitHubComment>> {
        // For incremental sync with since parameter, use sequential pagination
        // (smaller dataset, and we can't easily get total count)
        if since.is_some() {
            return self
                .list_comments_since_sequential(repo, since.unwrap())
                .await;
        }

        // Fetch first page to get total page count from Link header
        let (first_page_comments, total_pages) = self
            .fetch_comments_first_page_with_pagination_info(repo)
            .await?;

        if total_pages <= 1 {
            return Ok(FetchResult::complete(first_page_comments));
        }

        eprintln!("Fetching comments across {} pages...", total_pages);

        // Fetch remaining pages (2..=total_pages) in parallel with semaphore-bounded concurrency
        let futures = (2..=total_pages).map(|page| {
            let client = self.clone();
            let repo = repo.clone();
            async move {
                // Acquire semaphore permit before making request
                let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();
                client
                    .fetch_comments_page_with_retry(&repo, page, None)
                    .await
            }
        });

        // Stream results as they complete, printing progress
        let mut stream = stream::iter(futures).buffer_unordered(MAX_CONCURRENT_REQUESTS);
        let mut all_comments = first_page_comments;
        let mut error_count = 0;
        let mut rate_limit_errors = 0;
        let mut completed = 1; // Already have page 1

        while let Some(result) = stream.next().await {
            match result {
                Ok(comments) => all_comments.extend(comments),
                Err(e) => {
                    let err_str = e.to_string();
                    eprintln!("Warning: comments page fetch failed: {}", err_str);
                    if err_str.contains("rate limit") || err_str.contains("403") {
                        rate_limit_errors += 1;
                    }
                    error_count += 1;
                }
            }
            completed += 1;
            if completed % 10 == 0 || completed == total_pages {
                eprintln!(
                    "  {}/{} pages ({} comments)",
                    completed,
                    total_pages,
                    all_comments.len()
                );
            }
        }

        // Warn if we got partial results
        let is_complete = error_count == 0;
        if error_count > 0 && !all_comments.is_empty() {
            let reason = if rate_limit_errors > 0 {
                "rate limit"
            } else {
                "network error"
            };
            eprintln!(
                "Warning: {} of {} pages failed ({}), got {} comments",
                error_count,
                total_pages,
                reason,
                all_comments.len()
            );
        }

        Ok(FetchResult {
            items: all_comments,
            is_complete,
        })
    }

    /// Fetch comments updated since timestamp (sequential pagination for incremental sync)
    async fn list_comments_since_sequential(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<GitHubComment>> {
        let mut all_comments = Vec::new();
        let mut page = 1;

        loop {
            let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();
            match self
                .fetch_comments_page_with_retry(repo, page, Some(&since))
                .await
            {
                Ok(comments) => {
                    let count = comments.len();
                    all_comments.extend(comments);
                    if count < PER_PAGE {
                        break;
                    }
                    page += 1;
                }
                Err(e) => {
                    eprintln!("Warning: comments page {} fetch failed: {}", page, e);
                    return Ok(FetchResult::incomplete(all_comments));
                }
            }
        }

        Ok(FetchResult::complete(all_comments))
    }

    /// Fetch first page of comments and return total page count from Link header
    async fn fetch_comments_first_page_with_pagination_info(
        &self,
        repo: &Repo,
    ) -> Result<(Vec<GitHubComment>, usize)> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/comments?per_page={}&page=1",
            repo.owner, repo.name, PER_PAGE
        );

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();

            let response = match self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "isq")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) if attempt < MAX_RETRIES - 1 => {
                    let delay = Duration::from_secs(1 << attempt);
                    eprintln!(
                        "Network error fetching comments page 1, retrying in {:?}: {}",
                        delay, e
                    );
                    last_error = Some(e.to_string());
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            if response.status().is_success() {
                // Parse Link header to get total pages
                let total_pages = response
                    .headers()
                    .get("link")
                    .and_then(|h| h.to_str().ok())
                    .and_then(parse_last_page_from_link_header)
                    .unwrap_or(1);

                match response.json::<Vec<GitHubComment>>().await {
                    Ok(comments) => return Ok((comments, total_pages)),
                    Err(e) if attempt < MAX_RETRIES - 1 => {
                        let delay = Duration::from_secs(1 << attempt);
                        eprintln!("Decode error on comments page 1, retrying: {}", e);
                        last_error = Some(e.to_string());
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            let status = response.status().as_u16();
            let delay = get_retry_delay(&response, attempt);
            let body = response.text().await?;

            if is_rate_limited(status, &body) && attempt < MAX_RETRIES - 1 {
                eprintln!("Rate limited on comments page 1, retrying in {:?}", delay);
                tokio::time::sleep(delay).await;
                continue;
            }

            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        anyhow::bail!(
            "Max retries exceeded for comments page 1: {}",
            last_error.unwrap_or_default()
        )
    }

    /// Fetch a single page of comments with retry on rate limit
    async fn fetch_comments_page_with_retry(
        &self,
        repo: &Repo,
        page: usize,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<GitHubComment>> {
        let base_url = format!(
            "https://api.github.com/repos/{}/{}/issues/comments?per_page={}&page={}",
            repo.owner, repo.name, PER_PAGE, page
        );
        let url = match since {
            // Use Z suffix (not +00:00) to avoid URL encoding issues
            Some(ts) => format!(
                "{}&since={}",
                base_url,
                ts.to_rfc3339_opts(SecondsFormat::Secs, true)
            ),
            None => base_url,
        };

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            // Acquire semaphore permit before making request
            let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();

            let response = match self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token))
                .header("User-Agent", "isq")
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) if attempt < MAX_RETRIES - 1 => {
                    let delay = Duration::from_secs(1 << attempt);
                    eprintln!(
                        "Network error fetching comments page {}, retrying in {:?}: {}",
                        page, delay, e
                    );
                    last_error = Some(e.to_string());
                    tokio::time::sleep(delay).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            if response.status().is_success() {
                match response.json::<Vec<GitHubComment>>().await {
                    Ok(comments) => return Ok(comments),
                    Err(e) if attempt < MAX_RETRIES - 1 => {
                        let delay = Duration::from_secs(1 << attempt);
                        eprintln!("Decode error on comments page {}, retrying: {}", page, e);
                        last_error = Some(e.to_string());
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                }
            }

            let status = response.status().as_u16();
            let delay = get_retry_delay(&response, attempt);
            let body = response.text().await?;

            if is_rate_limited(status, &body) && attempt < MAX_RETRIES - 1 {
                eprintln!(
                    "Rate limited on comments page {}, retrying in {:?}",
                    page, delay
                );
                tokio::time::sleep(delay).await;
                continue;
            }

            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        anyhow::bail!(
            "Max retries exceeded for comments page {}: {}",
            page,
            last_error.unwrap_or_default()
        )
    }

    /// List all milestones (goals) for a repo
    pub async fn list_milestones(&self, repo: &Repo) -> Result<Vec<GitHubMilestone>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/milestones?state=all&per_page=100",
            repo.owner, repo.name
        );

        let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let milestones: Vec<GitHubMilestone> = response.json().await?;
        Ok(milestones)
    }

    /// Create a new milestone
    pub async fn create_milestone(
        &self,
        repo: &Repo,
        req: &CreateGoalRequest,
    ) -> Result<GitHubMilestone> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/milestones",
            repo.owner, repo.name
        );

        let mut body = serde_json::json!({
            "title": req.name,
        });

        if let Some(desc) = &req.description {
            body["description"] = serde_json::json!(desc);
        }

        if let Some(date) = &req.target_date {
            // GitHub needs full ISO 8601: append T00:00:00Z
            body["due_on"] = serde_json::json!(format!("{}T00:00:00Z", date));
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let milestone: GitHubMilestone = response.json().await?;
        Ok(milestone)
    }

    /// Close a milestone
    pub async fn close_milestone(&self, repo: &Repo, number: u64) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/milestones/{}",
            repo.owner, repo.name, number
        );

        let body = serde_json::json!({ "state": "closed" });

        let response = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Set milestone on an issue
    pub async fn set_issue_milestone(
        &self,
        repo: &Repo,
        issue_number: u64,
        milestone_number: u64,
    ) -> Result<()> {
        self.patch_issue(
            repo,
            issue_number,
            &serde_json::json!({ "milestone": milestone_number }),
        )
        .await
    }

    /// Internal add_label without auto-create (to avoid infinite recursion)
    pub async fn add_label_internal(
        &self,
        repo: &Repo,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "labels": [label] });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Create a label in the repository (internal, for add_label auto-create)
    pub async fn create_label_internal(&self, repo: &Repo, label: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/labels",
            repo.owner, repo.name
        );

        // Use a nice blue color for auto-created labels
        let payload = serde_json::json!({
            "name": label,
            "color": "1d76db",
            "description": "Auto-created by isq"
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        // 422 means label already exists, which is fine
        if response.status().is_success() || response.status().as_u16() == 422 {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("GitHub API error creating label {}: {}", status, body);
    }

    /// List all labels in the repository
    pub async fn list_labels(&self, repo: &Repo) -> Result<Vec<Label>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/labels?per_page=100",
            repo.owner, repo.name
        );

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error listing labels {}: {}", status, body);
        }

        let labels: Vec<GitHubLabel> = response.json().await?;
        Ok(labels
            .into_iter()
            .map(|l| Label::new(l.name, Some(l.color)))
            .collect())
    }

    /// Create a label in the repository
    pub async fn create_label(
        &self,
        repo: &Repo,
        name: &str,
        color: Option<&str>,
        description: Option<&str>,
    ) -> Result<Label> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/labels",
            repo.owner, repo.name
        );

        let color = color.unwrap_or("1d76db").trim_start_matches('#');
        let desc = description.unwrap_or("Created by isq");

        let payload = serde_json::json!({
            "name": name,
            "color": color,
            "description": desc
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error creating label {}: {}", status, body);
        }

        let label: GitHubLabel = response.json().await?;
        Ok(Label::new(label.name, Some(label.color)))
    }

    /// Add label to issue (public method that auto-creates if needed)
    pub async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "labels": [label] });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            return Ok(());
        }

        // Check if label doesn't exist (422 with "Label does not exist")
        let status = response.status();
        let body = response.text().await?;

        if status.as_u16() == 422 && body.to_lowercase().contains("label") {
            // Create the label and retry
            self.create_label_internal(repo, label).await?;
            return self.add_label_internal(repo, issue_number, label).await;
        }

        anyhow::bail!("GitHub API error {}: {}", status, body);
    }

    /// Remove label from issue
    pub async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/labels/{}",
            repo.owner, repo.name, issue_number, label
        );

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        // 404 is ok - label might not exist
        if !response.status().is_success() && response.status().as_u16() != 404 {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Assign user to issue
    pub async fn assign_issue(&self, repo: &Repo, issue_number: u64, assignee: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/assignees",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "assignees": [assignee] });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Create issue
    pub async fn create_issue(
        &self,
        repo: &Repo,
        req: &crate::forges::CreateIssueRequest,
    ) -> Result<crate::forges::Issue> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues",
            repo.owner, repo.name
        );

        let mut body = serde_json::json!({
            "title": req.title,
        });

        if let Some(b) = &req.body {
            body["body"] = serde_json::json!(b);
        }

        if !req.labels.is_empty() {
            body["labels"] = serde_json::json!(req.labels);
        }

        if let Some(goal_id) = &req.goal_id {
            if let Ok(milestone_num) = goal_id.parse::<u64>() {
                body["milestone"] = serde_json::json!(milestone_num);
            }
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let issue: GitHubIssue = response.json().await?;
        Ok(issue.into_issue())
    }

    /// Create comment on issue
    pub async fn create_comment(&self, repo: &Repo, issue_number: u64, body: &str) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/comments",
            repo.owner, repo.name, issue_number
        );

        let payload = serde_json::json!({ "body": body });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        Ok(())
    }

    /// Get rate limit info
    pub async fn get_rate_limit(&self) -> Result<Option<crate::forges::RateLimitInfo>> {
        let response = self
            .client
            .get("https://api.github.com/rate_limit")
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        #[derive(Deserialize)]
        struct RateLimitResponse {
            resources: Resources,
        }
        #[derive(Deserialize)]
        struct Resources {
            core: CoreLimit,
        }
        #[derive(Deserialize)]
        struct CoreLimit {
            limit: u32,
            remaining: u32,
            reset: i64,
        }

        let result: RateLimitResponse = response.json().await?;
        Ok(Some(crate::forges::RateLimitInfo {
            limit: result.resources.core.limit,
            remaining: result.resources.core.remaining,
            reset_at: result.resources.core.reset,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_link_header_with_last() {
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?per_page=100&page=2>; rel="next", <https://api.github.com/repos/foo/bar/issues/comments?per_page=100&page=5>; rel="last""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(5));
    }

    #[test]
    fn test_parse_link_header_last_first() {
        // GitHub sometimes returns rel="last" before rel="next"
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?page=10>; rel="last", <https://api.github.com/repos/foo/bar/issues/comments?page=2>; rel="next""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(10));
    }

    #[test]
    fn test_parse_link_header_no_last() {
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?page=2>; rel="next""#;
        assert_eq!(parse_last_page_from_link_header(header), None);
    }

    #[test]
    fn test_parse_link_header_empty() {
        assert_eq!(parse_last_page_from_link_header(""), None);
    }

    #[test]
    fn test_parse_link_header_large_page_number() {
        let header =
            r#"<https://api.github.com/repos/foo/bar/issues/comments?page=9999>; rel="last""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(9999));
    }

    #[test]
    fn test_parse_link_header_page_with_other_params() {
        // page= appears after other query params
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?per_page=100&since=2024-01-01&page=42>; rel="last""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(42));
    }
}
