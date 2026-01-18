//! GitHub milestone (goals) operations

use anyhow::Result;

use crate::forges::CreateGoalRequest;
use crate::repo::Repo;

use super::GitHubClient;
use super::rate_limit::{REQUEST_SEMAPHORE, throttle_write};
use super::types::GitHubMilestone;

impl GitHubClient {
    /// List all milestones (goals) for a repo
    pub async fn list_milestones(&self, repo: &Repo) -> Result<Vec<GitHubMilestone>> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/milestones?state=all&per_page=100",
            repo.owner, repo.name
        );

        let _permit = REQUEST_SEMAPHORE.acquire().await.unwrap();

        let response = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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
            .http_client()
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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
}
