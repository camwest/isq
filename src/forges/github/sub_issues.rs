//! GitHub sub-issues API operations

use std::collections::HashMap;

use anyhow::Result;
use futures::stream::{self, StreamExt};

use crate::repo::Repo;

use super::rate_limit::{MAX_CONCURRENT_REQUESTS, REQUEST_SEMAPHORE, throttle_write};
use super::types::GitHubParentIssue;
use super::GitHubClient;

impl GitHubClient {
    /// Get the parent issue number for an issue, if any
    /// Returns None if the issue has no parent (404 response)
    pub async fn get_issue_parent(&self, repo: &Repo, issue_number: u64) -> Result<Option<u64>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/parent",
            repo.owner, repo.name, issue_number
        );

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
            .header("User-Agent", "isq")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        if response.status().as_u16() == 404 {
            // No parent - this is expected for most issues
            return Ok(None);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub API error {}: {}", status, body);
        }

        let parent: GitHubParentIssue = response.json().await?;
        Ok(Some(parent.number))
    }

    /// Fetch parent info for multiple issues in parallel
    /// Returns a map of issue_number -> parent_number for issues that have parents
    pub async fn fetch_parents_for_issues(
        &self,
        repo: &Repo,
        issue_numbers: &[u64],
    ) -> HashMap<u64, u64> {
        if issue_numbers.is_empty() {
            return HashMap::new();
        }

        eprintln!("Fetching parent info for {} issues...", issue_numbers.len());

        let futures = issue_numbers.iter().copied().map(|issue_number| {
            let client = self.clone();
            let repo = repo.clone();
            async move {
                let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();
                match client.get_issue_parent(&repo, issue_number).await {
                    Ok(Some(parent)) => Some((issue_number, parent)),
                    Ok(None) => None,
                    Err(e) => {
                        // Log but don't fail - parent info is optional
                        eprintln!(
                            "Warning: failed to get parent for issue {}: {}",
                            issue_number, e
                        );
                        None
                    }
                }
            }
        });

        let stream = stream::iter(futures).buffer_unordered(MAX_CONCURRENT_REQUESTS);
        stream.filter_map(|x| async move { x }).collect().await
    }

    /// Add an issue as a sub-issue of a parent
    pub async fn add_sub_issue(&self, repo: &Repo, parent_number: u64, sub_issue_id: u64) -> Result<()> {
        throttle_write().await;

        let url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}/sub_issues",
            repo.owner, repo.name, parent_number
        );

        let payload = serde_json::json!({
            "sub_issue_id": sub_issue_id
        });

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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
}
