//! Forge trait definition for issue tracker abstraction.

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::types::{
    CreateGoalRequest, CreateIssueRequest, FetchResult, Goal, Issue, Label, RateLimitInfo,
};
use crate::db;
use crate::repo::Repo;

/// Abstraction over GitHub/GitLab/Forgejo APIs
///
/// CLI code should use this trait, not forge-specific implementations directly.
/// This enables adding new backends without changing CLI code.
#[async_trait]
pub trait Forge: Send + Sync {
    /// List all open issues for a repo (full fetch)
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>>;

    /// List issues updated since timestamp (incremental fetch)
    async fn list_issues_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<Issue>>;

    /// Create a new issue
    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue>;

    /// Add a comment to an issue
    async fn create_comment(&self, repo: &Repo, issue_id: &str, body: &str) -> Result<()>;

    /// Close an issue
    async fn close_issue(&self, repo: &Repo, issue_id: &str) -> Result<()>;

    /// Reopen an issue
    async fn reopen_issue(&self, repo: &Repo, issue_id: &str) -> Result<()>;

    /// Add a label to an issue
    async fn add_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()>;

    /// Remove a label from an issue
    async fn remove_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()>;

    /// Assign a user to an issue
    async fn assign_issue(&self, repo: &Repo, issue_id: &str, assignee: &str) -> Result<()>;

    /// List all comments for a repo (full fetch)
    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<db::Comment>>;

    /// List comments updated since timestamp (incremental fetch)
    async fn list_comments_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<db::Comment>>;

    /// List all goals (GitHub: milestones, Linear: projects)
    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>>;

    /// Create a new goal
    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal>;

    /// Close a goal
    async fn close_goal(&self, repo: &Repo, goal_id: &str) -> Result<()>;

    /// Assign an issue to a goal
    async fn assign_to_goal(&self, repo: &Repo, issue_id: &str, goal_id: &str) -> Result<()>;

    /// Get rate limit status (returns None if forge doesn't have rate limits)
    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>>;

    /// List all labels in the repo
    async fn list_labels(&self, repo: &Repo) -> Result<Vec<Label>>;

    /// Create a label in the repo
    async fn create_label(
        &self,
        repo: &Repo,
        name: &str,
        color: Option<&str>,
        description: Option<&str>,
    ) -> Result<Label>;

    /// Handle on_start lifecycle event for an issue
    /// Each forge interprets the config according to its own schema
    /// username is provided for assign_self functionality
    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        username: Option<&str>,
    ) -> Result<()>;

    /// Validate on_start config before use
    /// Returns error with helpful message if config is invalid
    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()>;

    /// Handle on_cleanup lifecycle event for an issue
    /// Each forge interprets the config according to its own schema
    /// username is provided for potential unassignment functionality
    async fn handle_on_cleanup(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        username: Option<&str>,
    ) -> Result<()>;

    /// Validate on_cleanup config before use
    /// Returns error with helpful message if config is invalid
    ///
    /// Note: Currently unused because cleanup is best-effort (invalid config
    /// should not prevent worktree removal). Kept for API symmetry with
    /// validate_on_start_config and potential future use in a config check command.
    #[allow(dead_code)]
    fn validate_on_cleanup_config(&self, config: &toml::Value) -> Result<()>;

    /// Apply forge-specific priority configuration to issues.
    /// Called after list_issues to enrich priority data based on repo config.
    ///
    /// Default: no-op. Override if forge uses config-based priority (e.g., GitHub labels).
    /// Linear uses native priority, so it doesn't need to override this.
    fn apply_priority_config(&self, _issues: &mut [Issue], _config: &toml::Value) {
        // Default: do nothing
    }

    /// Handle forge-specific commands (e.g., list-fields for JIRA)
    ///
    /// Default: returns error saying no commands available.
    /// Override in forge implementations that have specific commands.
    async fn handle_command(&self, command: &str, _args: &[String]) -> Result<()> {
        Err(anyhow!("Unknown command: {}", command))
    }

    /// Query issues with forge-specific options (e.g., JQL for JIRA).
    ///
    /// Returns `Ok(Some(issues))` if the forge handles the options directly,
    /// or `Ok(None)` to fall back to the local cache.
    ///
    /// This is an escape hatch for forge-specific query languages that can't
    /// be translated to local SQL filters.
    async fn query_issues_with_opts(
        &self,
        _repo: &Repo,
        _opts: &std::collections::HashMap<String, String>,
    ) -> Result<Option<Vec<Issue>>> {
        Ok(None) // Default: use cache
    }
}
