//! GitHub API client implementation

use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::stream::{self, StreamExt};

use crate::forges::FetchResult;
use crate::repo::Repo;

use super::rate_limit::{
    MAX_CONCURRENT_REQUESTS, MAX_RETRIES, PER_PAGE, REQUEST_SEMAPHORE, get_retry_delay,
    is_rate_limited,
};
use super::types::{GitHubIssue, SearchResult};

/// Create HTTP client with appropriate settings
pub fn create_http_client() -> reqwest::Client {
    crate::forges::create_http_client()
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

    /// Get the HTTP client (for use by submodules)
    pub(super) fn http_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Get the token (for use by submodules)
    pub(super) fn token(&self) -> &str {
        &self.token
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
        if let Some(since) = since {
            return self.list_issues_since_sequential(repo, since).await;
        }

        // Get total count from search API
        let total = self.get_issue_count(repo).await?;

        if total == 0 {
            return Ok(FetchResult::complete(Vec::new()));
        }

        let total_pages = total.div_ceil(PER_PAGE);
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

        // Fetch parent info for all issues (sub-issues API)
        let issue_numbers: Vec<u64> = all_issues
            .iter()
            .filter_map(|i| i.id.parse().ok())
            .collect();
        let parents = self.fetch_parents_for_issues(repo, &issue_numbers).await;

        // Update issues with parent info
        for issue in &mut all_issues {
            if let Ok(num) = issue.id.parse::<u64>() {
                if let Some(&parent_num) = parents.get(&num) {
                    issue.parent_id = Some(parent_num.to_string());
                }
            }
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

        // Fetch parent info for updated issues (sub-issues API)
        if !all_issues.is_empty() {
            let issue_numbers: Vec<u64> = all_issues
                .iter()
                .filter_map(|i| i.id.parse().ok())
                .collect();
            let parents = self.fetch_parents_for_issues(repo, &issue_numbers).await;

            for issue in &mut all_issues {
                if let Ok(num) = issue.id.parse::<u64>() {
                    if let Some(&parent_num) = parents.get(&num) {
                        issue.parent_id = Some(parent_num.to_string());
                    }
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
}
