use anyhow::Result;
use async_trait::async_trait;
use futures::future::join_all;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

use crate::forge::{CreateIssueRequest, Forge};
use crate::repo::Repo;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub user: User,
    pub labels: Vec<Label>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
}

/// GitHub API comment response (for deserializing)
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubComment {
    pub id: u64,
    pub issue_url: String,
    pub body: String,
    pub user: User,
    pub created_at: String,
}

impl GitHubComment {
    /// Parse issue number from issue_url (e.g., "https://api.github.com/repos/owner/repo/issues/123")
    pub fn issue_number(&self) -> Option<u64> {
        self.issue_url.rsplit('/').next()?.parse().ok()
    }
}

#[derive(Clone)]
pub struct GitHubClient {
    client: reqwest::Client,
    token: String,
}

#[derive(Deserialize)]
struct SearchResult {
    total_count: usize,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token,
        }
    }

    /// Fetch all open issues for a repo (parallel pagination with rate limiting)
    pub async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>> {
        // Get total count from search API
        let total = self.get_issue_count(repo).await?;

        if total == 0 {
            return Ok(Vec::new());
        }

        let total_pages = (total + PER_PAGE - 1) / PER_PAGE;
        eprintln!("Fetching {} issues across {} pages...", total, total_pages);

        // Fetch all pages in parallel with semaphore-bounded concurrency
        let futures: Vec<_> = (1..=total_pages)
            .map(|page| {
                let client = self.clone();
                let repo = repo.clone();
                async move {
                    // Acquire semaphore permit before making request
                    let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();
                    client.fetch_page_with_retry(&repo, page).await
                }
            })
            .collect();

        let results = join_all(futures).await;

        let mut all_issues = Vec::with_capacity(total);
        for result in results {
            match result {
                Ok(issues) => all_issues.extend(issues),
                Err(e) => eprintln!("Warning: page fetch failed: {}", e),
            }
        }

        Ok(all_issues)
    }

    /// Get total open issue count via search API
    async fn get_issue_count(&self, repo: &Repo) -> Result<usize> {
        let url = format!(
            "https://api.github.com/search/issues?q=repo:{}/{}+state:open+is:issue&per_page=1",
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
    async fn fetch_page_with_retry(&self, repo: &Repo, page: usize) -> Result<Vec<Issue>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open&per_page={}&page={}",
            repo.owner, repo.name, PER_PAGE, page
        );

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
                match response.json::<Vec<Issue>>().await {
                    Ok(issues) => return Ok(issues),
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

        let user: User = response.json().await?;
        Ok(user.login)
    }

    /// Fetch a single issue by number
    async fn fetch_issue(&self, repo: &Repo, number: u64) -> Result<Issue> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}",
            repo.owner, repo.name, number
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
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let issue: Issue = response.json().await?;
        Ok(issue)
    }

    /// Helper for PATCH requests to update issue state
    async fn patch_issue(&self, repo: &Repo, number: u64, body: &serde_json::Value) -> Result<()> {
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
    pub async fn list_all_comments(&self, repo: &Repo) -> Result<Vec<GitHubComment>> {
        // Start with page 1 and fetch until empty
        let mut all_comments = Vec::new();
        let mut page = 1;

        loop {
            let comments = self.fetch_comments_page_with_retry(repo, page).await?;
            let is_empty = comments.is_empty();
            all_comments.extend(comments);

            if is_empty {
                break;
            }
            page += 1;
        }

        Ok(all_comments)
    }

    /// Fetch a single page of comments with retry on rate limit
    async fn fetch_comments_page_with_retry(&self, repo: &Repo, page: usize) -> Result<Vec<GitHubComment>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/comments?per_page={}&page={}",
            repo.owner, repo.name, PER_PAGE, page
        );

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
                eprintln!("Rate limited on comments page {}, retrying in {:?}", page, delay);
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
}

#[async_trait]
impl Forge for GitHubClient {
    async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>> {
        self.list_issues(repo).await
    }

    async fn get_issue(&self, repo: &Repo, number: u64) -> Result<Issue> {
        self.fetch_issue(repo, number).await
    }

    async fn get_user(&self) -> Result<String> {
        self.get_user().await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
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

        let issue: Issue = response.json().await?;
        Ok(issue)
    }

    async fn create_comment(&self, repo: &Repo, issue_number: u64, body: &str) -> Result<()> {
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

    async fn close_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "closed" }))
            .await
    }

    async fn reopen_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "open" }))
            .await
    }

    async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
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

    async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
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

    async fn assign_issue(&self, repo: &Repo, issue_number: u64, assignee: &str) -> Result<()> {
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

    async fn list_all_comments(&self, repo: &Repo) -> Result<Vec<crate::db::Comment>> {
        let github_comments = GitHubClient::list_all_comments(self, repo).await?;

        // Convert GitHubComment to db::Comment
        let comments: Vec<crate::db::Comment> = github_comments
            .into_iter()
            .filter_map(|c| {
                Some(crate::db::Comment {
                    comment_id: c.id.to_string(),
                    issue_number: c.issue_number()?,
                    body: c.body,
                    author: c.user.login,
                    created_at: c.created_at,
                })
            })
            .collect();

        Ok(comments)
    }
}
