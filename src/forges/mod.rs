mod github;
mod linear;

use std::process::Command;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::credentials;
use crate::db;
use crate::repo::Repo;

pub use github::GitHubClient;
pub use linear::LinearClient;

// ============================================================================
// Auth Configuration
// ============================================================================

/// Authentication configuration for a forge.
///
/// Each forge defines its auth config as a const. The auth logic is generic
/// and works with any AuthConfig, following the open/closed principle.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Service name for keyring storage (e.g., "github", "linear")
    pub keyring_service: &'static str,
    /// Environment variable name for token fallback
    pub env_var: &'static str,
    /// CLI command to get token, if any (e.g., &["gh", "auth", "token"])
    pub cli_command: Option<&'static [&'static str]>,
    /// Human-readable forge name for error messages
    pub display_name: &'static str,
    /// Command to authenticate (shown in error messages)
    pub link_command: &'static str,
}

impl AuthConfig {
    /// Get a token using the fallback chain: CLI → keyring → env var
    pub fn get_token(&self) -> Result<String> {
        // 1. Try CLI command if configured
        if let Some(cmd) = self.cli_command {
            if let Ok(token) = self.try_cli_token(cmd) {
                return Ok(token);
            }
        }

        // 2. Try stored credentials from OS keyring
        if let Ok(Some(cred)) = credentials::get_credential(self.keyring_service) {
            return Ok(cred.access_token);
        }

        // 3. Try environment variable
        if let Ok(token) = std::env::var(self.env_var) {
            return Ok(token);
        }

        // No token available - build helpful error message
        Err(self.auth_error())
    }

    /// Check if credentials are available (without detailed errors)
    pub fn has_credentials(&self) -> bool {
        // Check CLI
        if let Some(cmd) = self.cli_command {
            if self.try_cli_token(cmd).is_ok() {
                return true;
            }
        }

        // Check keyring
        if let Ok(Some(_)) = credentials::get_credential(self.keyring_service) {
            return true;
        }

        // Check env var
        std::env::var(self.env_var).is_ok()
    }

    /// Store a credential in the OS keyring
    pub fn store_credential(
        &self,
        access_token: &str,
        refresh_token: Option<&str>,
        expires_at: Option<&str>,
    ) -> Result<()> {
        credentials::set_credential(self.keyring_service, access_token, refresh_token, expires_at)
    }

    /// Get the full credential (including refresh token) from keyring
    pub fn get_credential(&self) -> Result<Option<credentials::Credential>> {
        credentials::get_credential(self.keyring_service)
    }

    /// Try to get a token from a CLI command
    fn try_cli_token(&self, cmd: &[&str]) -> Result<String> {
        let output = Command::new(cmd[0])
            .args(&cmd[1..])
            .output()
            .map_err(|_| anyhow!("{} CLI not found", self.display_name))?;

        if !output.status.success() {
            return Err(anyhow!("{} CLI not authenticated", self.display_name));
        }

        let token = String::from_utf8(output.stdout)?.trim().to_string();
        if token.is_empty() {
            return Err(anyhow!("{} CLI returned empty token", self.display_name));
        }

        Ok(token)
    }

    /// Build a helpful error message when no auth is available
    fn auth_error(&self) -> anyhow::Error {
        let mut msg = format!("{} not authenticated.\n\n", self.display_name);

        let mut option = 1;

        // CLI option (if available)
        if let Some(cmd) = self.cli_command {
            msg.push_str(&format!(
                "Option {}: Install {} CLI and authenticate\n",
                option, cmd[0]
            ));
            option += 1;
        }

        // OAuth option
        msg.push_str(&format!("Option {}: Run: {}\n", option, self.link_command));
        option += 1;

        // Env var option
        msg.push_str(&format!(
            "Option {}: Set {} environment variable",
            option, self.env_var
        ));

        anyhow!(msg)
    }
}

// ============================================================================
// Issue Types
// ============================================================================

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

/// All supported forge types (for iteration)
pub const ALL_FORGE_TYPES: &[ForgeType] = &[ForgeType::GitHub, ForgeType::Linear];

/// Common OAuth token type returned by all forge OAuth flows
#[derive(Debug, Clone)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

// ============================================================================
// Link Types
// ============================================================================

/// Arguments for the link command, parsed from CLI options
#[derive(Debug, Clone, Default)]
pub struct LinkArgs {
    pub team: Option<String>,
    pub list_teams: bool,
}

impl LinkArgs {
    /// Parse from CLI -o key=value options
    pub fn parse(opts: &[String]) -> Result<Self> {
        let mut args = Self::default();
        for opt in opts {
            if opt == "list-teams" {
                args.list_teams = true;
            } else if let Some((key, value)) = opt.split_once('=') {
                match key {
                    "team" => args.team = Some(value.to_string()),
                    _ => return Err(anyhow!("Unknown option: {}", key)),
                }
            } else {
                return Err(anyhow!("Invalid option format: {}. Use key=value or flag name.", opt));
            }
        }
        Ok(args)
    }
}

/// Result of a successful link operation
#[derive(Debug, Clone)]
pub struct LinkResult {
    pub forge_repo: String,
    pub display_name: String,
    pub issues: Vec<Issue>,
}

/// Generate error message for repos not linked to a forge
pub fn not_linked_error() -> anyhow::Error {
    let forges: Vec<_> = ALL_FORGE_TYPES.iter().map(|f| format!("  isq link {}", f.as_str())).collect();
    anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n{}", forges.join("\n"))
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

    /// Get auth configuration for this forge
    pub fn auth(&self) -> &'static AuthConfig {
        match self {
            ForgeType::GitHub => &github::AUTH,
            ForgeType::Linear => &linear::AUTH,
        }
    }

    /// Run OAuth flow for this forge
    pub async fn oauth_flow(&self) -> Result<OAuthToken> {
        match self {
            ForgeType::GitHub => {
                let token = github::oauth_flow().await?;
                Ok(OAuthToken {
                    access_token: token.access_token,
                    refresh_token: token.refresh_token,
                    expires_in: None, // GitHub tokens don't expire
                })
            }
            ForgeType::Linear => {
                let token = linear::oauth_flow().await?;
                Ok(OAuthToken {
                    access_token: token.access_token,
                    refresh_token: token.refresh_token,
                    expires_in: token.expires_in,
                })
            }
        }
    }

    /// Create a forge client from a token
    pub fn create_client(&self, token: String) -> Box<dyn Forge> {
        match self {
            ForgeType::GitHub => Box::new(GitHubClient::new(token)),
            ForgeType::Linear => Box::new(LinearClient::new(token)),
        }
    }

    /// Run the complete link flow for this forge
    pub async fn link(&self, repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
        match self {
            ForgeType::GitHub => github::link(repo_path, args).await,
            ForgeType::Linear => linear::link(repo_path, args).await,
        }
    }
}

/// Request to create an issue
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub goal_id: Option<String>,
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
    let token = github::AUTH.get_token()?;
    Ok(Box::new(GitHubClient::new(token)))
}

/// Get the forge for a specific repo path, looking up the link in the database.
///
/// Returns an error if the repo is not linked to a forge.
pub fn get_forge_for_repo(repo_path: &str) -> Result<(Box<dyn Forge>, db::RepoLink)> {
    let conn = db::open()?;
    let link = db::get_repo_link(&conn, repo_path)?
        .ok_or_else(not_linked_error)?;

    let forge_type = ForgeType::from_str(&link.forge_type)
        .ok_or_else(|| anyhow!("Unknown forge type: {}", link.forge_type))?;

    let forge: Box<dyn Forge> = match forge_type {
        ForgeType::GitHub => {
            let token = github::AUTH.get_token()?;
            Box::new(GitHubClient::new(token))
        }
        ForgeType::Linear => {
            let token = linear::AUTH.get_token()?;
            Box::new(LinearClient::new(token))
        }
    };

    Ok((forge, link))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    // Helper to temporarily set/unset env vars
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            // SAFETY: Tests run serially via #[serial], so no concurrent access
            unsafe { env::set_var(key, value) };
            Self { key, original }
        }

        fn unset(key: &'static str) -> Self {
            let original = env::var(key).ok();
            // SAFETY: Tests run serially via #[serial], so no concurrent access
            unsafe { env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: Tests run serially via #[serial], so no concurrent access
            match &self.original {
                Some(val) => unsafe { env::set_var(self.key, val) },
                None => unsafe { env::remove_var(self.key) },
            }
        }
    }

    // Test AuthConfig for a mock forge
    const TEST_AUTH: AuthConfig = AuthConfig {
        keyring_service: "_isq_test",
        env_var: "_ISQ_TEST_TOKEN",
        cli_command: None,
        display_name: "Test",
        link_command: "isq link test",
    };

    #[test]
    #[serial]
    fn test_auth_config_env_var_fallback() {
        let _guard = EnvGuard::set("_ISQ_TEST_TOKEN", "test_token_123");

        let result = TEST_AUTH.get_token();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test_token_123");
    }

    #[test]
    #[serial]
    fn test_auth_config_has_credentials_with_env_var() {
        let _guard = EnvGuard::set("_ISQ_TEST_TOKEN", "test_token");
        assert!(TEST_AUTH.has_credentials());
    }

    #[test]
    #[serial]
    fn test_auth_config_has_credentials_without_anything() {
        let _guard = EnvGuard::unset("_ISQ_TEST_TOKEN");
        // May still be true if keyring has credentials, but shouldn't panic
        let _ = TEST_AUTH.has_credentials();
    }

    #[test]
    #[serial]
    fn test_auth_config_error_message() {
        let _guard = EnvGuard::unset("_ISQ_TEST_TOKEN");

        let result = TEST_AUTH.get_token();

        // If it fails (no keyring, no env var), check error message
        if result.is_err() {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Test not authenticated"));
            assert!(err.contains("isq link test"));
            assert!(err.contains("_ISQ_TEST_TOKEN"));
        }
    }

    #[test]
    fn test_github_auth_config() {
        // Verify GitHub AUTH is properly configured
        assert_eq!(github::AUTH.keyring_service, "github");
        assert_eq!(github::AUTH.env_var, "GITHUB_TOKEN");
        assert!(github::AUTH.cli_command.is_some());
        assert_eq!(github::AUTH.display_name, "GitHub");
    }

    #[test]
    fn test_linear_auth_config() {
        // Verify Linear AUTH is properly configured
        assert_eq!(linear::AUTH.keyring_service, "linear");
        assert_eq!(linear::AUTH.env_var, "LINEAR_API_KEY");
        assert!(linear::AUTH.cli_command.is_none());
        assert_eq!(linear::AUTH.display_name, "Linear");
    }

    #[test]
    #[serial]
    fn test_github_token_from_env_var() {
        let _guard = EnvGuard::set("GITHUB_TOKEN", "ghp_test123");

        let result = github::AUTH.get_token();
        // May succeed with env var, or may use gh CLI if available
        if result.is_ok() {
            assert!(!result.unwrap().is_empty());
        }
    }

    #[test]
    #[serial]
    fn test_linear_token_from_env_var() {
        let _guard = EnvGuard::set("LINEAR_API_KEY", "lin_test456");

        let result = linear::AUTH.get_token();
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }
}
