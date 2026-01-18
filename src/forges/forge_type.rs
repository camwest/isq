//! ForgeType enum and factory functions.

use anyhow::{Result, anyhow};

use super::auth::AuthConfig;
use super::link::{LinkArgs, LinkResult, not_linked_error};
use super::traits::Forge;
use super::{GitHubClient, JiraClient, LinearClient};
use super::{github, jira, linear};
use crate::db;

/// Supported forge types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeType {
    GitHub,
    Jira,
    Linear,
}

/// All supported forge types (for iteration)
pub const ALL_FORGE_TYPES: &[ForgeType] = &[ForgeType::GitHub, ForgeType::Jira, ForgeType::Linear];

impl ForgeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ForgeType::GitHub => "github",
            ForgeType::Jira => "jira",
            ForgeType::Linear => "linear",
        }
    }

    pub fn from_str(s: &str) -> Option<ForgeType> {
        match s.to_lowercase().as_str() {
            "github" => Some(ForgeType::GitHub),
            "jira" => Some(ForgeType::Jira),
            "linear" => Some(ForgeType::Linear),
            _ => None,
        }
    }

    /// Get auth configuration for this forge
    pub fn auth(&self) -> &'static AuthConfig {
        match self {
            ForgeType::GitHub => &github::AUTH,
            ForgeType::Jira => &jira::AUTH,
            ForgeType::Linear => &linear::AUTH,
        }
    }

    /// Get default [on_start] TOML config for this forge
    pub fn default_on_start_toml(&self) -> &'static str {
        match self {
            ForgeType::GitHub => github::DEFAULT_ON_START_TOML,
            ForgeType::Jira => jira::DEFAULT_ON_START_TOML,
            ForgeType::Linear => linear::DEFAULT_ON_START_TOML,
        }
    }

    /// Get default [on_cleanup] TOML config for this forge
    pub fn default_on_cleanup_toml(&self) -> &'static str {
        match self {
            ForgeType::GitHub => github::DEFAULT_ON_CLEANUP_TOML,
            ForgeType::Jira => jira::DEFAULT_ON_CLEANUP_TOML,
            ForgeType::Linear => linear::DEFAULT_ON_CLEANUP_TOML,
        }
    }

    /// Run the complete link flow for this forge
    pub async fn link(&self, repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
        match self {
            ForgeType::GitHub => github::link(repo_path, args).await,
            ForgeType::Jira => jira::link(repo_path, args).await,
            ForgeType::Linear => linear::link(repo_path, args).await,
        }
    }

    /// Get available forge-specific commands
    pub fn available_commands(&self) -> Vec<&'static str> {
        match self {
            ForgeType::Jira => vec!["list-fields"],
            _ => vec![],
        }
    }
}

/// Get the forge for a specific repo path, looking up the link in the database.
///
/// Returns an error if the repo is not linked to a forge.
pub fn get_forge_for_repo(repo_path: &str) -> Result<(Box<dyn Forge>, db::RepoLink)> {
    let conn = db::open()?;
    let link = db::get_repo_link(&conn, repo_path)?.ok_or_else(not_linked_error)?;

    let forge_type = ForgeType::from_str(&link.forge_type)
        .ok_or_else(|| anyhow!("Unknown forge type: {}", link.forge_type))?;

    let forge: Box<dyn Forge> = match forge_type {
        ForgeType::GitHub => {
            let token = github::AUTH.get_token()?;
            Box::new(GitHubClient::new(token))
        }
        ForgeType::Jira => {
            let creds = jira::get_credentials_for_repo(&link.forge_repo)?;
            Box::new(JiraClient::new(creds))
        }
        ForgeType::Linear => {
            let token = linear::AUTH.get_token()?;
            Box::new(LinearClient::new(token))
        }
    };

    Ok((forge, link))
}
