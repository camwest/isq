//! GitHub forge implementation
//!
//! This module provides GitHub API integration for issue tracking.

pub mod client;
mod comments;
mod graphql;
mod labels;
mod link;
mod milestones;
mod mutations;
pub mod oauth;
mod priority;
pub mod rate_limit;
mod sub_issues;
pub mod types;

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::forges::{
    AuthConfig, CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, Goal, Issue, Label,
    RateLimitInfo, UpdateIssueRequest,
};
use crate::repo::Repo;

pub use client::GitHubClient;
pub use link::link;

// ============================================================================
// Auth Configuration
// ============================================================================

/// GitHub authentication configuration
pub const AUTH: AuthConfig = AuthConfig {
    keyring_service: "github",
    env_var: "GITHUB_TOKEN",
    cli_command: Some(&["gh", "auth", "token"]),
    display_name: "GitHub",
    link_command: "isq link github",
};

/// Default [on_start] config for GitHub repos
pub const DEFAULT_ON_START_TOML: &str = "add_labels = [\"in progress\"]\nassign_self = true\n";

/// Default [on_cleanup] config for GitHub repos
pub const DEFAULT_ON_CLEANUP_TOML: &str = "remove_labels = [\"in progress\"]\n";

// ============================================================================
// On-Start Configuration
// ============================================================================

/// GitHub-specific on_start configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct GitHubOnStartConfig {
    /// Labels to add to the issue
    #[serde(default)]
    add_labels: Vec<String>,
    /// Assign the issue to yourself
    #[serde(default)]
    assign_self: bool,
}

/// GitHub-specific on_cleanup configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct GitHubOnCleanupConfig {
    /// Labels to remove from the issue
    #[serde(default)]
    remove_labels: Vec<String>,
}

// ============================================================================
// Forge Trait Implementation
// ============================================================================

#[async_trait]
impl Forge for GitHubClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        // Use GraphQL for efficient fetching with parent info inline
        self.list_issues_graphql(repo, None).await
    }

    async fn list_issues_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<Issue>> {
        // Use GraphQL for efficient fetching with parent info inline
        self.list_issues_graphql(repo, Some(since)).await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        self.create_issue(repo, &req).await
    }

    async fn create_comment(&self, repo: &Repo, issue_id: &str, body: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.create_comment(repo, issue_number, body).await
    }

    async fn close_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.patch_issue(
            repo,
            issue_number,
            &serde_json::json!({ "state": "closed" }),
        )
        .await
    }

    async fn reopen_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "open" }))
            .await
    }

    async fn update_issue(
        &self,
        repo: &Repo,
        issue_id: &str,
        req: UpdateIssueRequest,
    ) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;

        if req.priority.is_some() {
            anyhow::bail!(
                "GitHub does not support native issue priority updates. Use labels configured in isq.toml."
            );
        }

        let mut body = serde_json::Map::new();
        if let Some(title) = req.title {
            body.insert("title".to_string(), serde_json::json!(title));
        }
        if let Some(description) = req.body {
            body.insert("body".to_string(), serde_json::json!(description));
        }

        if body.is_empty() {
            anyhow::bail!("No fields provided to update");
        }

        self.patch_issue(repo, issue_number, &serde_json::Value::Object(body))
            .await
    }

    async fn add_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.add_label(repo, issue_number, label).await
    }

    async fn remove_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.remove_label(repo, issue_number, label).await
    }

    async fn assign_issue(&self, repo: &Repo, issue_id: &str, assignee: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.assign_issue(repo, issue_number, assignee).await
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<crate::db::Comment>> {
        let result = self.list_all_comments_internal(repo, None).await?;

        // Convert GitHubComment to db::Comment
        let comments: Vec<crate::db::Comment> = result
            .items
            .into_iter()
            .filter_map(|c| {
                Some(crate::db::Comment {
                    comment_id: c.id.to_string(),
                    issue_id: c.issue_id()?,
                    body: c.body,
                    author: c.user.login,
                    created_at: c.created_at,
                    updated_at: Some(c.updated_at),
                })
            })
            .collect();

        Ok(FetchResult {
            items: comments,
            is_complete: result.is_complete,
        })
    }

    async fn list_comments_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<crate::db::Comment>> {
        let result = self.list_all_comments_internal(repo, Some(since)).await?;

        // Convert GitHubComment to db::Comment
        let comments: Vec<crate::db::Comment> = result
            .items
            .into_iter()
            .filter_map(|c| {
                Some(crate::db::Comment {
                    comment_id: c.id.to_string(),
                    issue_id: c.issue_id()?,
                    body: c.body,
                    author: c.user.login,
                    created_at: c.created_at,
                    updated_at: Some(c.updated_at),
                })
            })
            .collect();

        Ok(FetchResult {
            items: comments,
            is_complete: result.is_complete,
        })
    }

    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        let milestones = self.list_milestones(repo).await?;
        Ok(milestones.into_iter().map(Goal::from).collect())
    }

    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal> {
        let milestone = self.create_milestone(repo, &req).await?;
        Ok(Goal::from(milestone))
    }

    async fn close_goal(&self, repo: &Repo, goal_id: &str) -> Result<()> {
        let number: u64 = goal_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid milestone number: {}", goal_id))?;
        self.close_milestone(repo, number).await
    }

    async fn assign_to_goal(&self, repo: &Repo, issue_id: &str, goal_id: &str) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        let milestone_number: u64 = goal_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid milestone number: {}", goal_id))?;
        self.set_issue_milestone(repo, issue_number, milestone_number)
            .await
    }

    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        self.get_rate_limit().await
    }

    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        username: Option<&str>,
    ) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;

        // Parse GitHub-specific config from opaque toml::Value
        let cfg: GitHubOnStartConfig = config.clone().try_into().unwrap_or_default();

        // Add each configured label
        for label in &cfg.add_labels {
            self.add_label(repo, issue_number, label).await?;
        }

        // Assign to self if configured
        if cfg.assign_self
            && let Some(user) = username
        {
            self.assign_issue(repo, issue_number, user).await?;
        }

        Ok(())
    }

    async fn list_labels(&self, repo: &Repo) -> Result<Vec<Label>> {
        self.list_labels(repo).await
    }

    async fn create_label(
        &self,
        repo: &Repo,
        name: &str,
        color: Option<&str>,
        description: Option<&str>,
    ) -> Result<Label> {
        self.create_label(repo, name, color, description).await
    }

    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()> {
        let _: GitHubOnStartConfig = config.clone().try_into().context(
            "Invalid [on_start] config for GitHub.\nValid fields: add_labels, assign_self",
        )?;
        Ok(())
    }

    async fn handle_on_cleanup(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        _username: Option<&str>,
    ) -> Result<()> {
        let issue_number: u64 = issue_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;

        // Parse GitHub-specific config from opaque toml::Value
        let cfg: GitHubOnCleanupConfig = config.clone().try_into().unwrap_or_default();

        // Remove each configured label (ignore errors - label might not exist)
        for label in &cfg.remove_labels {
            if let Err(e) = self.remove_label(repo, issue_number, label).await {
                eprintln!("Warning: could not remove label '{}': {}", label, e);
            }
        }

        Ok(())
    }

    fn validate_on_cleanup_config(&self, config: &toml::Value) -> Result<()> {
        let _: GitHubOnCleanupConfig = config
            .clone()
            .try_into()
            .context("Invalid [on_cleanup] config for GitHub.\nValid fields: remove_labels")?;
        Ok(())
    }

    fn apply_priority_config(&self, issues: &mut [Issue], config: &toml::Value) {
        priority::apply_priority_from_labels(issues, config);
    }
}
