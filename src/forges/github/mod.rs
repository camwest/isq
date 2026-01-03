//! GitHub forge implementation
//!
//! This module provides GitHub API integration for issue tracking.

pub mod client;
pub mod oauth;
pub mod types;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::forges::{
    AuthConfig, CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, ForgeType, Goal,
    Issue, Label, LinkArgs, LinkResult, RateLimitInfo,
};
use crate::repo::Repo;
use crate::{config, db, repo};

pub use client::GitHubClient;
pub use oauth::oauth_flow;

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
pub const DEFAULT_ON_START_TOML: &str =
    "add_labels = [\"in progress\"]\nassign_self = true\n";

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

// ============================================================================
// Link Flow
// ============================================================================

/// Run the complete GitHub link flow.
/// Handles auth, verifies credentials, syncs issues, and returns the result.
pub async fn link(repo_path: &str, _args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::GitHub;
    let conn = db::open()?;

    // Detect GitHub repo from git remote
    let repo = repo::detect_repo()?;

    // Try existing auth first, fall back to OAuth
    let (token, auth_method) = match AUTH.get_token() {
        Ok(t) => {
            // Store in keychain so daemon can access
            // (gh CLI isn't available from launchd)
            AUTH.store_credential(&t, None, None)?;
            (t, "existing")
        }
        Err(_) => {
            let oauth_token = oauth_flow().await?;
            AUTH.store_credential(
                &oauth_token.access_token,
                oauth_token.refresh_token.as_deref(),
                None, // GitHub tokens don't expire by default
            )?;
            (oauth_token.access_token, "OAuth")
        }
    };

    let client = GitHubClient::new(token);

    // Verify authentication
    let username = client.get_user().await?;
    println!("✓ Authenticated as {} (via {})", username, auth_method);

    // Sync issues
    let display_name = repo.full_name();
    println!("Syncing {}...", display_name);
    let issues_result = client.list_issues_internal(&repo, None).await?;

    // Save to database (for GitHub, username serves as both user_id and user_name)
    db::set_repo_link(
        &conn,
        repo_path,
        forge_type.as_str(),
        &repo.full_name(),
        Some(&display_name),
        Some(&username),
        Some(&username),
    )?;
    db::save_issues(
        &conn,
        &repo.full_name(),
        &issues_result.items,
        true,
        issues_result.is_complete,
    )?;
    db::add_watched_repo(&conn, repo_path)?;

    // Create .config/isq.toml with defaults
    if config::create_repo_config(std::path::Path::new(repo_path), forge_type.as_str())? {
        println!("✓ Created .config/isq.toml");
    }

    // Install commit hook
    match repo::install_hook(std::path::Path::new(repo_path)) {
        Ok(true) => println!("✓ Installed commit hook"),
        Ok(false) => {} // Already installed, silent
        Err(e) => eprintln!("Warning: Could not install hook: {}", e),
    }

    println!("✓ Cached {} issues", issues_result.items.len());

    Ok(LinkResult { display_name })
}

// ============================================================================
// Priority Configuration
// ============================================================================

/// Apply priority from label configuration to issues.
/// This is a pure function extracted for testability.
fn apply_priority_from_labels(issues: &mut [Issue], config: &toml::Value) {
    // Parse priority config: { "P0" = 0, "bug" = 1, ... }
    let priority_labels: std::collections::HashMap<String, u8> = config
        .as_table()
        .map(|table| {
            table
                .iter()
                .filter_map(|(label, value)| {
                    let priority = value.as_integer()?;
                    if (0..=4).contains(&priority) {
                        Some((label.clone(), priority as u8))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if priority_labels.is_empty() {
        return;
    }

    // Apply priority from labels to issues
    for issue in issues.iter_mut() {
        // Only apply if priority hasn't been set (default is 4/none)
        if issue.priority == 4 {
            // Find the highest priority label (lowest number)
            let mut best_priority = 4u8;
            let mut best_label: Option<String> = None;

            for label in &issue.labels {
                if let Some(&priority) = priority_labels.get(&label.name) {
                    if priority < best_priority {
                        best_priority = priority;
                        best_label = Some(label.name.clone());
                    }
                }
            }

            if best_priority < 4 {
                issue.priority = best_priority;
                issue.priority_label = best_label;
            }
        }
    }
}

// ============================================================================
// Forge Trait Implementation
// ============================================================================

#[async_trait]
impl Forge for GitHubClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, None).await
    }

    async fn list_issues_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, Some(since)).await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        self.create_issue(repo, &req).await
    }

    async fn create_comment(&self, repo: &Repo, issue_id: &str, body: &str) -> Result<()> {
        let issue_number: u64 = issue_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.create_comment(repo, issue_number, body).await
    }

    async fn close_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number: u64 = issue_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "closed" }))
            .await
    }

    async fn reopen_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number: u64 = issue_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.patch_issue(repo, issue_number, &serde_json::json!({ "state": "open" }))
            .await
    }

    async fn add_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number: u64 = issue_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.add_label(repo, issue_number, label).await
    }

    async fn remove_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number: u64 = issue_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;
        self.remove_label(repo, issue_number, label).await
    }

    async fn assign_issue(&self, repo: &Repo, issue_id: &str, assignee: &str) -> Result<()> {
        let issue_number: u64 = issue_id.parse()
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
        let issue_number: u64 = issue_id.parse()
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
        let issue_number: u64 = issue_id.parse()
            .map_err(|_| anyhow::anyhow!("Invalid issue number: {}", issue_id))?;

        // Parse GitHub-specific config from opaque toml::Value
        let cfg: GitHubOnStartConfig = config.clone().try_into().unwrap_or_default();

        // Add each configured label
        for label in &cfg.add_labels {
            self.add_label(repo, issue_number, label).await?;
        }

        // Assign to self if configured
        if cfg.assign_self {
            if let Some(user) = username {
                self.assign_issue(repo, issue_number, user).await?;
            }
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
        let _: GitHubOnStartConfig = config
            .clone()
            .try_into()
            .context("Invalid [on_start] config for GitHub.\nValid fields: add_labels, assign_self")?;
        Ok(())
    }

    fn apply_priority_config(&self, issues: &mut [Issue], config: &toml::Value) {
        apply_priority_from_labels(issues, config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_issue(id: &str, labels: Vec<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            title: format!("Issue {}", id),
            body: None,
            state: "open".to_string(),
            author: "testuser".to_string(),
            labels: labels
                .into_iter()
                .map(|s| Label::name_only(s.to_string()))
                .collect(),
            assignees: vec![],
            priority: 4, // Default: none
            priority_label: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            url: None,
            milestone: None,
        }
    }

    #[test]
    fn test_apply_priority_from_labels() {
        let config: toml::Value = toml::from_str(
            r#"
            P0 = 0
            P1 = 1
            P2 = 2
        "#,
        )
        .unwrap();

        let mut issues = vec![make_issue("1", vec!["P0", "bug"])];
        apply_priority_from_labels(&mut issues, &config);

        assert_eq!(issues[0].priority, 0);
        assert_eq!(issues[0].priority_label, Some("P0".to_string()));
    }

    #[test]
    fn test_priority_uses_lowest_value() {
        let config: toml::Value = toml::from_str(
            r#"
            P0 = 0
            P1 = 1
        "#,
        )
        .unwrap();

        // Issue has both P1 (priority 1) and P0 (priority 0) - should pick P0
        let mut issues = vec![make_issue("1", vec!["P1", "P0"])];
        apply_priority_from_labels(&mut issues, &config);

        assert_eq!(issues[0].priority, 0);
        assert_eq!(issues[0].priority_label, Some("P0".to_string()));
    }

    #[test]
    fn test_priority_no_matching_labels() {
        let config: toml::Value = toml::from_str(
            r#"
            P0 = 0
            P1 = 1
        "#,
        )
        .unwrap();

        let mut issues = vec![make_issue("1", vec!["bug", "enhancement"])];
        apply_priority_from_labels(&mut issues, &config);

        // Should remain at default priority
        assert_eq!(issues[0].priority, 4);
        assert_eq!(issues[0].priority_label, None);
    }

    #[test]
    fn test_priority_empty_config() {
        let config: toml::Value = toml::from_str("").unwrap();

        let mut issues = vec![make_issue("1", vec!["P0"])];
        apply_priority_from_labels(&mut issues, &config);

        // No config means no priority mapping
        assert_eq!(issues[0].priority, 4);
    }

    #[test]
    fn test_priority_invalid_config_values() {
        let config: toml::Value = toml::from_str(
            r#"
            P0 = 0
            bad = 99
            negative = -1
        "#,
        )
        .unwrap();

        // P0 should work, but bad/negative should be ignored
        let mut issues = vec![make_issue("1", vec!["P0"]), make_issue("2", vec!["bad"])];
        apply_priority_from_labels(&mut issues, &config);

        assert_eq!(issues[0].priority, 0); // P0 works
        assert_eq!(issues[1].priority, 4); // bad value ignored
    }
}
