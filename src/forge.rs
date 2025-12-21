use anyhow::Result;
use async_trait::async_trait;

use crate::auth;
use crate::github::{GitHubClient, Issue};
use crate::repo::Repo;

/// Request to create an issue
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
}

/// Abstraction over GitHub/GitLab/Forgejo APIs
///
/// CLI code should use this trait, not forge-specific implementations directly.
/// This enables adding new backends without changing CLI code.
#[async_trait]
pub trait Forge: Send + Sync {
    /// List all open issues for a repo
    async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>>;

    /// Get a single issue by number
    async fn get_issue(&self, repo: &Repo, number: u64) -> Result<Issue>;

    /// Get authenticated user's login
    async fn get_user(&self) -> Result<String>;

    /// Create a new issue
    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue>;

    /// Add a comment to an issue
    async fn create_comment(&self, repo: &Repo, issue_number: u64, body: &str) -> Result<()>;

    /// Close an issue
    async fn close_issue(&self, repo: &Repo, issue_number: u64) -> Result<()>;

    /// Reopen an issue
    async fn reopen_issue(&self, repo: &Repo, issue_number: u64) -> Result<()>;

    /// Add a label to an issue
    async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()>;

    /// Remove a label from an issue
    async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()>;

    /// Assign a user to an issue
    async fn assign_issue(&self, repo: &Repo, issue_number: u64, assignee: &str) -> Result<()>;
}

/// Get the appropriate forge for the current context.
/// Currently always returns GitHub; will detect GitLab/Forgejo from remote URL later.
pub fn get_forge() -> Result<Box<dyn Forge>> {
    let token = auth::get_gh_token()?;
    Ok(Box::new(GitHubClient::new(token)))
}
