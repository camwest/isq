//! GitHub sub-issues API operations (REST API for mutations)

use anyhow::Result;

use crate::repo::Repo;

use super::GitHubClient;
use super::rate_limit::throttle_write;

impl GitHubClient {
    /// Add an issue as a sub-issue of a parent
    pub async fn add_sub_issue(
        &self,
        repo: &Repo,
        parent_number: u64,
        sub_issue_id: u64,
    ) -> Result<()> {
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
