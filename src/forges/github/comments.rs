//! GitHub comment fetching operations

use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use futures::stream::{self, StreamExt};

use crate::forges::FetchResult;
use crate::repo::Repo;

use super::GitHubClient;
use super::rate_limit::{
    MAX_CONCURRENT_REQUESTS, MAX_RETRIES, PER_PAGE, REQUEST_SEMAPHORE, get_retry_delay,
    is_rate_limited, parse_last_page_from_link_header,
};
use super::types::GitHubComment;

impl GitHubClient {
    /// Fetch all comments for a repo (parallel pagination with rate limiting)
    /// Uses repo-level endpoint: GET /repos/{owner}/{repo}/issues/comments
    pub async fn list_all_comments_internal(
        &self,
        repo: &Repo,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<GitHubComment>> {
        // For incremental sync with since parameter, use sequential pagination
        // (smaller dataset, and we can't easily get total count)
        if let Some(since_time) = since {
            return self.list_comments_since_sequential(repo, since_time).await;
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
                .http_client()
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token()))
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
    pub(super) async fn fetch_comments_page_with_retry(
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
                .http_client()
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.token()))
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
}
