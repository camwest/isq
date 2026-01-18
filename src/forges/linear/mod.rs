//! Linear issue tracker integration
//!
//! This module provides Linear API client functionality including:
//! - OAuth PKCE authentication flow
//! - Team and issue management
//! - Project (goal) operations
//! - Comment syncing

mod client;
mod oauth;
mod types;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub use client::LinearClient;
pub use oauth::oauth_flow;
#[allow(unused_imports)]
pub use oauth::{TokenResponse, refresh_token};
#[allow(unused_imports)]
pub use types::{LinearOrganization, LinearProject, LinearTeam};

use super::{
    AuthConfig, CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, ForgeType, Goal, Issue,
    Label, LinkArgs, LinkResult, RateLimitInfo,
};
use crate::repo::Repo;
use crate::{config, db, repo};

// ============================================================================
// Configuration
// ============================================================================

/// Linear authentication configuration
pub const AUTH: AuthConfig = AuthConfig {
    keyring_service: "linear",
    env_var: "LINEAR_API_KEY",
    cli_command: None, // Linear has no CLI
    display_name: "Linear",
    link_command: "isq link linear",
};

/// Default [on_start] config for Linear repos
/// Uses stable type "started" (works regardless of custom state names)
pub const DEFAULT_ON_START_TOML: &str = "transition = \"started\"\nassign_self = true\n";

/// Default [on_cleanup] config for Linear repos
/// Commented out by default since moving issues back to backlog may not be desired
pub const DEFAULT_ON_CLEANUP_TOML: &str =
    "# transition = \"backlog\"  # Optional: move issue back to backlog\n";

// API endpoints
pub(super) const GRAPHQL_URL: &str = "https://api.linear.app/graphql";

// OAuth configuration
pub(super) const LINEAR_CLIENT_ID: &str = "a6c010f01947bd3b847cb3c1707366e5";
pub(super) const LINEAR_AUTH_URL: &str = "https://linear.app/oauth/authorize";
pub(super) const LINEAR_TOKEN_URL: &str = "https://api.linear.app/oauth/token";
pub(super) const REDIRECT_PORT: u16 = 19284;
pub(super) const REDIRECT_URI: &str = "http://127.0.0.1:19284/callback";

// ============================================================================
// Helper Functions
// ============================================================================

/// Map Linear priority to our priority scale.
/// Linear: 0=none, 1=urgent, 2=high, 3=normal, 4=low
/// Ours:   0=urgent, 1=high, 2=medium, 3=low, 4=none
pub(super) fn map_linear_priority(linear_priority: u8) -> u8 {
    match linear_priority {
        0 => 4, // no priority → none
        1 => 0, // urgent → urgent
        2 => 1, // high → high
        3 => 2, // normal → medium
        4 => 3, // low → low
        _ => 4, // unknown → none
    }
}

/// Parse Linear's rate limit reset timestamp.
/// Linear returns milliseconds, we need seconds.
fn parse_reset_timestamp(value: &str) -> Option<i64> {
    value.parse::<i64>().ok().map(|ms| ms / 1000)
}

/// Linear-specific on_start configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LinearOnStartConfig {
    /// Workflow state to transition to (type like "started" or name like "In Progress")
    transition: Option<String>,
    /// Assign the issue to yourself
    #[serde(default)]
    assign_self: bool,
}

/// Linear-specific on_cleanup configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LinearOnCleanupConfig {
    /// Workflow state to transition to (type like "backlog" or name like "Todo")
    transition: Option<String>,
}

/// URL encoding helper
pub(super) mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }
}

/// Parse a Linear issue identifier (e.g., "DEV-123") and extract the numeric part
fn parse_issue_number(issue_id: &str) -> anyhow::Result<u64> {
    // Linear identifiers are in the format "KEY-123"
    issue_id
        .rsplit('-')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| anyhow!("Invalid Linear issue identifier: {}", issue_id))
}

// ============================================================================
// Link Flow
// ============================================================================

/// Run the complete Linear link flow.
/// Handles auth, team selection, syncs issues, and returns the result.
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::Linear;
    let conn = db::open()?;

    // Try existing auth first, fall back to OAuth
    let (token, is_new_auth) = match AUTH.get_token() {
        Ok(t) => (t, false),
        Err(_) => {
            let oauth_token = oauth_flow().await?;
            let expires_at = oauth_token.expires_in.map(|secs| {
                (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339()
            });
            AUTH.store_credential(
                &oauth_token.access_token,
                oauth_token.refresh_token.as_deref(),
                expires_at.as_deref(),
            )?;
            (oauth_token.access_token, true)
        }
    };

    let client = LinearClient::new(token);

    // Verify authentication - get user ID for assignment and display name for printing
    let (user_id, user_display_name) = client.get_viewer().await?;
    if is_new_auth {
        println!("✓ Authenticated as {}", user_display_name);
    }

    // List teams
    let teams = client.list_teams().await?;
    if teams.is_empty() {
        anyhow::bail!("No teams found in your Linear workspace");
    }

    // Handle -o list-teams flag
    if args.has_flag("list-teams") {
        println!("Available teams:");
        for team in &teams {
            println!("  {} ({})", team.name, team.key);
        }
        // Return empty result for list-teams (caller should not save)
        return Err(anyhow!("-o list-teams: showing available teams"));
    }

    // Resolve team from -o team=X argument or auto-select if only one
    let team = if let Some(team_query) = args.get("team") {
        let query_lower = team_query.to_lowercase();
        teams
            .iter()
            .find(|t| t.name.to_lowercase() == query_lower || t.key.to_lowercase() == query_lower)
            .ok_or_else(|| {
                let available: Vec<_> = teams
                    .iter()
                    .map(|t| format!("{} ({})", t.name, t.key))
                    .collect();
                anyhow!(
                    "Team '{}' not found.\n\nAvailable teams:\n  {}",
                    team_query,
                    available.join("\n  ")
                )
            })?
    } else if teams.len() == 1 {
        println!("Using team: {} ({})", teams[0].name, teams[0].key);
        &teams[0]
    } else {
        let available: Vec<_> = teams
            .iter()
            .map(|t| format!("{} ({})", t.name, t.key))
            .collect();
        anyhow::bail!(
            "Multiple teams available. Specify one with -o team=<name>.\n\nAvailable teams:\n  {}\n\nExample: isq link linear -o team=\"{}\"",
            available.join("\n  "),
            teams[0].name
        );
    };

    // Get organization info for display name
    let org = client.get_organization().await?;
    let display_name = format!("{}/{}", org.url_key, team.key);
    let forge_repo = format!("{}/{}", team.key, team.id);

    // Create pseudo-repo for syncing (unused but kept for future reference)
    let _pseudo_repo = repo::Repo {
        owner: team.key.clone(),
        name: team.id.clone(),
    };

    // Sync issues
    println!("Syncing {}...", team.name);
    let issues_result = client.list_team_issues_internal(&team.id, None).await?;

    // Save to database (user_id for API calls, user_display_name for --mine filter)
    db::set_repo_link(
        &conn,
        repo_path,
        forge_type.as_str(),
        &forge_repo,
        Some(&display_name),
        Some(&user_id),
        Some(&user_display_name),
    )?;
    db::save_issues(
        &conn,
        &forge_repo,
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

    Ok(LinkResult {
        display_name: team.name.clone(),
    })
}

// ============================================================================
// Forge Trait Implementation
// ============================================================================

#[async_trait]
impl Forge for LinearClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        // For Linear, repo.owner is ignored and repo.name is the team ID
        self.list_team_issues_internal(&repo.name, None).await
    }

    async fn list_issues_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<Issue>> {
        self.list_team_issues_internal(&repo.name, Some(since))
            .await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        let team_id = &repo.name;
        let org = self.get_organization().await?;

        // Get label IDs if any labels specified (Linear requires empty array, not null)
        let label_ids = if !req.labels.is_empty() {
            self.get_label_ids(team_id, &req.labels).await?
        } else {
            Vec::new()
        };

        // Build mutation dynamically based on whether projectId is provided
        let (query, variables) = if let Some(project_id) = &req.goal_id {
            let q = r#"
                mutation($teamId: String!, $title: String!, $description: String, $labelIds: [String!], $projectId: String!) {
                    issueCreate(input: { teamId: $teamId, title: $title, description: $description, labelIds: $labelIds, projectId: $projectId }) {
                        issue {
                            id
                            identifier
                            number
                            title
                        }
                    }
                }
            "#;
            let v = serde_json::json!({
                "teamId": team_id,
                "title": req.title,
                "description": req.body,
                "labelIds": label_ids,
                "projectId": project_id
            });
            (q, v)
        } else {
            let q = r#"
                mutation($teamId: String!, $title: String!, $description: String, $labelIds: [String!]) {
                    issueCreate(input: { teamId: $teamId, title: $title, description: $description, labelIds: $labelIds }) {
                        issue {
                            id
                            identifier
                            number
                            title
                        }
                    }
                }
            "#;
            let v = serde_json::json!({
                "teamId": team_id,
                "title": req.title,
                "description": req.body,
                "labelIds": label_ids
            });
            (q, v)
        };

        let response: types::IssueCreateResponse = self.query(query, Some(variables)).await?;
        let created = response.issue_create.issue;
        let url = format!(
            "https://linear.app/{}/issue/{}",
            org.url_key, created.identifier
        );

        Ok(Issue {
            id: created.identifier,
            title: created.title,
            body: req.body,
            state: "open".to_string(),
            author: "me".to_string(),
            labels: req.labels.into_iter().map(Label::name_only).collect(),
            assignees: vec![], // Not returned by mutation
            priority: 4,       // Default: none (not returned by mutation)
            priority_label: None,
            created_at: String::new(), // Not returned by mutation
            updated_at: String::new(),
            url: Some(url),
            milestone: req.goal_id.clone(),
        })
    }

    async fn create_comment(&self, repo: &Repo, issue_id: &str, body: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;

        let query = r#"
            mutation($issueId: String!, $body: String!) {
                commentCreate(input: { issueId: $issueId, body: $body }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "body": body
        });

        let response: types::CommentCreateResponse = self.query(query, Some(variables)).await?;
        if !response.comment_create.success {
            anyhow::bail!("Failed to create comment");
        }
        Ok(())
    }

    async fn close_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        let done_state = self.get_state_by_type(&repo.name, "completed").await?;

        let query = r#"
            mutation($issueId: String!, $stateId: String!) {
                issueUpdate(id: $issueId, input: { stateId: $stateId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "stateId": done_state.id
        });

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to close issue");
        }
        Ok(())
    }

    async fn reopen_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        // Try "backlog" first, fall back to "unstarted" or "started"
        let backlog_state = match self.get_state_by_type(&repo.name, "backlog").await {
            Ok(state) => state,
            Err(_) => match self.get_state_by_type(&repo.name, "unstarted").await {
                Ok(state) => state,
                Err(_) => self.get_state_by_type(&repo.name, "started").await?,
            },
        };

        let query = r#"
            mutation($issueId: String!, $stateId: String!) {
                issueUpdate(id: $issueId, input: { stateId: $stateId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "stateId": backlog_state.id
        });

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to reopen issue");
        }
        Ok(())
    }

    async fn add_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        let label_ids = self.get_label_ids(&repo.name, &[label.to_string()]).await?;

        if label_ids.is_empty() {
            anyhow::bail!("Label '{}' not found", label);
        }

        // Get current label IDs and add the new one
        let mut current_ids: Vec<String> =
            issue.labels.nodes.iter().map(|l| l.id.clone()).collect();
        if !current_ids.contains(&label_ids[0]) {
            current_ids.push(label_ids[0].clone());
        }

        let query = r#"
            mutation($issueId: String!, $labelIds: [String!]!) {
                issueUpdate(id: $issueId, input: { labelIds: $labelIds }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "labelIds": current_ids
        });

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to add label");
        }
        Ok(())
    }

    async fn remove_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;

        // Get current label IDs and remove the specified one
        let label_lower = label.to_lowercase();
        let new_ids: Vec<String> = issue
            .labels
            .nodes
            .iter()
            .filter(|l| l.name.to_lowercase() != label_lower)
            .map(|l| l.id.clone())
            .collect();

        let query = r#"
            mutation($issueId: String!, $labelIds: [String!]!) {
                issueUpdate(id: $issueId, input: { labelIds: $labelIds }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "labelIds": new_ids
        });

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to remove label");
        }
        Ok(())
    }

    async fn assign_issue(&self, repo: &Repo, issue_id: &str, assignee: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        // CLI path: look up user by name/email to get their ID
        let user = self.get_user_by_name(assignee).await?;
        self.assign_issue_by_id(&repo.name, issue_number, &user.id)
            .await
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<crate::db::Comment>> {
        self.list_comments_internal(&repo.name, None).await
    }

    async fn list_comments_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<crate::db::Comment>> {
        self.list_comments_internal(&repo.name, Some(since)).await
    }

    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        let projects = self.list_projects(&repo.name).await?;
        Ok(projects.into_iter().map(Goal::from).collect())
    }

    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal> {
        let project = self.create_project(&repo.name, &req).await?;
        Ok(Goal::from(project))
    }

    async fn close_goal(&self, _repo: &Repo, goal_id: &str) -> Result<()> {
        self.complete_project(goal_id).await
    }

    async fn assign_to_goal(&self, repo: &Repo, issue_id: &str, goal_id: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        // Get the internal issue ID from the issue number
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        self.set_issue_project(&issue.id, goal_id).await
    }

    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        // Linear returns rate limit info in response headers
        // Make a minimal query to get the headers
        let request = types::GraphQLRequest {
            query: "query { viewer { id } }".to_string(),
            variables: None,
        };

        let token = self.token.read().unwrap().clone();
        let client = super::create_http_client();
        let response = client
            .post(GRAPHQL_URL)
            .header("Authorization", &token)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        // Extract rate limit headers
        // Linear uses: X-RateLimit-Requests-Limit, X-RateLimit-Requests-Remaining, X-RateLimit-Requests-Reset
        let limit = response
            .headers()
            .get("x-ratelimit-requests-limit")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1500); // Default to Linear's documented limit

        let remaining = response
            .headers()
            .get("x-ratelimit-requests-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok());

        let reset_at = response
            .headers()
            .get("x-ratelimit-requests-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_reset_timestamp);

        match (remaining, reset_at) {
            (Some(remaining), Some(reset_at)) => Ok(Some(RateLimitInfo {
                limit,
                remaining,
                reset_at,
            })),
            _ => Ok(None), // Headers not present, Linear may not always send them
        }
    }

    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        user_id: Option<&str>,
    ) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;

        // Parse Linear-specific config from opaque toml::Value
        let cfg: LinearOnStartConfig = config.clone().try_into().unwrap_or_default();

        // Transition to configured workflow state
        if let Some(ref transition) = cfg.transition {
            self.transition_issue(&repo.name, issue_number, transition)
                .await?;
        }

        // Assign to self if configured (user_id is the Linear user UUID)
        if cfg.assign_self {
            if let Some(id) = user_id {
                self.assign_issue_by_id(&repo.name, issue_number, id)
                    .await?;
            }
        }

        Ok(())
    }

    async fn list_labels(&self, repo: &Repo) -> Result<Vec<Label>> {
        // For Linear, repo.name is the team ID
        let query = r#"
            query($teamId: ID!) {
                team(id: $teamId) {
                    labels {
                        nodes {
                            name
                            color
                        }
                    }
                }
            }
        "#;

        let variables = serde_json::json!({ "teamId": repo.name });

        #[derive(Deserialize)]
        struct TeamLabelsResponse {
            team: TeamLabels,
        }
        #[derive(Deserialize)]
        struct TeamLabels {
            labels: LabelNodes,
        }
        #[derive(Deserialize)]
        struct LabelNodes {
            nodes: Vec<types::LinearLabel>,
        }

        let response: TeamLabelsResponse = self.query(query, Some(variables)).await?;
        Ok(response
            .team
            .labels
            .nodes
            .into_iter()
            .map(|l| Label::new(l.name, Some(l.color)))
            .collect())
    }

    async fn create_label(
        &self,
        repo: &Repo,
        name: &str,
        color: Option<&str>,
        _description: Option<&str>,
    ) -> Result<Label> {
        // For Linear, repo.name is the team ID
        let query = r#"
            mutation($teamId: String!, $name: String!, $color: String) {
                issueLabelCreate(input: { teamId: $teamId, name: $name, color: $color }) {
                    issueLabel {
                        name
                        color
                    }
                }
            }
        "#;

        let color = color.map(|c| c.trim_start_matches('#').to_string());

        let variables = serde_json::json!({
            "teamId": repo.name,
            "name": name,
            "color": color
        });

        #[derive(Deserialize)]
        struct CreateLabelResponse {
            #[serde(rename = "issueLabelCreate")]
            issue_label_create: IssueLabelCreate,
        }
        #[derive(Deserialize)]
        struct IssueLabelCreate {
            #[serde(rename = "issueLabel")]
            issue_label: types::LinearLabel,
        }

        let response: CreateLabelResponse = self.query(query, Some(variables)).await?;
        let label = response.issue_label_create.issue_label;
        Ok(Label::new(label.name, Some(label.color)))
    }

    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()> {
        let _: LinearOnStartConfig = config.clone().try_into().context(
            "Invalid [on_start] config for Linear.\nValid fields: transition, assign_self",
        )?;
        Ok(())
    }

    async fn handle_on_cleanup(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        _user_id: Option<&str>,
    ) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;

        // Parse Linear-specific config from opaque toml::Value
        let cfg: LinearOnCleanupConfig = config.clone().try_into().unwrap_or_default();

        // Transition to configured workflow state (optional)
        if let Some(ref transition) = cfg.transition {
            if let Err(e) = self
                .transition_issue(&repo.name, issue_number, transition)
                .await
            {
                eprintln!("Warning: could not transition issue: {}", e);
            }
        }

        Ok(())
    }

    fn validate_on_cleanup_config(&self, config: &toml::Value) -> Result<()> {
        let _: LinearOnCleanupConfig = config
            .clone()
            .try_into()
            .context("Invalid [on_cleanup] config for Linear.\nValid fields: transition")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_linear_priority() {
        // Linear 0 (no priority) -> our 4 (none)
        assert_eq!(map_linear_priority(0), 4);
        // Linear 1 (urgent) -> our 0 (urgent)
        assert_eq!(map_linear_priority(1), 0);
        // Linear 2 (high) -> our 1 (high)
        assert_eq!(map_linear_priority(2), 1);
        // Linear 3 (medium) -> our 2 (medium)
        assert_eq!(map_linear_priority(3), 2);
        // Linear 4 (low) -> our 3 (low)
        assert_eq!(map_linear_priority(4), 3);
        // Unknown -> none
        assert_eq!(map_linear_priority(5), 4);
        assert_eq!(map_linear_priority(255), 4);
    }

    #[test]
    fn test_parse_reset_timestamp_converts_ms_to_seconds() {
        // Linear returns milliseconds, we need seconds
        // 1736640000000 ms = 1736640000 seconds (Jan 2025)
        assert_eq!(parse_reset_timestamp("1736640000000"), Some(1736640000));

        // Verify it actually divides by 1000
        assert_eq!(parse_reset_timestamp("5000"), Some(5));
        assert_eq!(parse_reset_timestamp("1000"), Some(1));
        assert_eq!(parse_reset_timestamp("999"), Some(0)); // rounds down

        // Invalid input
        assert_eq!(parse_reset_timestamp("not_a_number"), None);
        assert_eq!(parse_reset_timestamp(""), None);
    }
}
