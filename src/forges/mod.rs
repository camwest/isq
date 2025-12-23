pub mod github;
pub mod linear;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::db;
use crate::repo::Repo;

pub use github::GitHubClient;
pub use linear::LinearClient;

/// Forge-agnostic issue representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: String,
    pub labels: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub url: Option<String>,
    /// Goal name (GitHub: milestone title, Linear: project name)
    pub milestone: Option<String>,
}

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

/// Goal state (normalized across forges)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalState {
    Open,
    Closed,
}

impl GoalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalState::Open => "open",
            GoalState::Closed => "closed",
        }
    }

    pub fn from_str(s: &str) -> GoalState {
        match s.to_lowercase().as_str() {
            "closed" | "completed" | "canceled" => GoalState::Closed,
            _ => GoalState::Open,
        }
    }
}

/// A time-bound container for issues (GitHub: Milestone, Linear: Project)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
    pub state: GoalState,
    /// Progress as a fraction (0.0 to 1.0), always available
    pub progress: f64,
    /// Open issue count, if forge provides it efficiently
    pub open_count: Option<u64>,
    /// Closed issue count, if forge provides it efficiently
    pub closed_count: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: Option<String>,
}

/// Request to create a goal
pub struct CreateGoalRequest {
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
}

/// Rate limit status from a forge
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    pub remaining: u32,
    pub reset_at: i64, // Unix timestamp
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

    /// List all comments for a repo (batch operation for sync)
    async fn list_all_comments(&self, repo: &Repo) -> Result<Vec<db::Comment>>;

    /// List all goals (GitHub: milestones, Linear: projects)
    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>>;

    /// Create a new goal
    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal>;

    /// Close a goal
    async fn close_goal(&self, repo: &Repo, goal_id: &str) -> Result<()>;

    /// Assign an issue to a goal
    async fn assign_to_goal(&self, repo: &Repo, issue_number: u64, goal_id: &str) -> Result<()>;

    /// Get rate limit status (returns None if forge doesn't have rate limits)
    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>>;
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
