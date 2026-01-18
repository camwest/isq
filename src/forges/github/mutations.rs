//! GitHub issue mutation operations (create, update, comment, assign)

use anyhow::Result;
use serde::Deserialize;

use crate::forges::RateLimitInfo;
use crate::repo::Repo;

use super::GitHubClient;
use super::rate_limit::throttle_write;
use super::types::GitHubIssue;

impl GitHubClient {
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
            .http_client()
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token()))
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

    /// Get authenticated user's login
    pub async fn get_user(&self) -> Result<String> {
        let response = self
            .http_client()
            .get("https://api.github.com/user")
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

        #[derive(Deserialize)]
        struct User {
            login: String,
        }
        let user: User = response.json().await?;
        Ok(user.login)
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

        if let Some(goal_id) = &req.goal_id
            && let Ok(milestone_num) = goal_id.parse::<u64>()
        {
            body["milestone"] = serde_json::json!(milestone_num);
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

        let gh_issue: GitHubIssue = response.json().await?;

        // If parent_id is provided, add this issue as a sub-issue
        if let Some(parent_id) = &req.parent_id
            && let Ok(parent_number) = parent_id.parse::<u64>()
        {
            self.add_sub_issue(repo, parent_number, gh_issue.id).await?;
        }

        let mut issue = gh_issue.into_issue();
        issue.parent_id = req.parent_id.clone();
        Ok(issue)
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

    /// Get rate limit info
    pub async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        let response = self
            .http_client()
            .get("https://api.github.com/rate_limit")
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
        Ok(Some(RateLimitInfo {
            limit: result.resources.core.limit,
            remaining: result.resources.core.remaining,
            reset_at: result.resources.core.reset,
        }))
    }
}
