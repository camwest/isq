use anyhow::Result;
use async_trait::async_trait;

use crate::auth;
use crate::db;
use crate::github::{GitHubClient, Issue};
use crate::linear::LinearClient;
use crate::repo::Repo;

/// Supported forge types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeType {
    GitHub,
    Linear,
}

impl ForgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ForgeType::GitHub => "github",
            ForgeType::Linear => "linear",
        }
    }

    pub fn from_str(s: &str) -> Option<ForgeType> {
        match s.to_lowercase().as_str() {
            "github" => Some(ForgeType::GitHub),
            "linear" => Some(ForgeType::Linear),
            _ => None,
        }
    }
}

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
    let token = auth::get_github_token()?;
    Ok(Box::new(GitHubClient::new(token)))
}

/// Get the forge for a specific repo path, looking up the link in the database.
///
/// Returns an error if the repo is not linked to a forge.
pub fn get_forge_for_repo(repo_path: &str) -> Result<(Box<dyn Forge>, db::RepoLink)> {
    let conn = db::open()?;
    let link = db::get_repo_link(&conn, repo_path)?
        .ok_or_else(|| anyhow::anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n  isq link github\n  isq link linear"))?;

    let forge_type = ForgeType::from_str(&link.forge_type)
        .ok_or_else(|| anyhow::anyhow!("Unknown forge type: {}", link.forge_type))?;

    let forge: Box<dyn Forge> = match forge_type {
        ForgeType::GitHub => {
            let token = auth::get_github_token()?;
            Box::new(GitHubClient::new(token))
        }
        ForgeType::Linear => {
            let token = auth::get_linear_token()?;
            Box::new(LinearClient::new(token))
        }
    };

    Ok((forge, link))
}
