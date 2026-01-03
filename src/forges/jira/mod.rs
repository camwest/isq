//! JIRA issue tracker integration
//!
//! This module provides JIRA Cloud API client functionality including:
//! - OAuth 2.0 PKCE authentication flow
//! - API token authentication for CI/headless use
//! - Issue and project management
//! - Version (goal) operations
//! - Comment syncing
//! - ADF (Atlassian Document Format) conversion

mod adf;
mod client;
mod oauth;
mod types;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub use client::JiraClient;
pub use oauth::{get_accessible_resources, get_credentials_from_env, get_stored_credentials, oauth_flow, store_credentials, JiraAuthMode, JiraCredentials};
#[allow(unused_imports)]
pub use oauth::{refresh_token, AccessibleResource, TokenResponse};
#[allow(unused_imports)]
pub use types::{JiraProject, JiraUser, JiraVersion};

use super::{AuthConfig, CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, ForgeType, Goal, GoalState, Issue, Label, LinkArgs, LinkResult, RateLimitInfo};
use crate::repo::Repo;
use crate::{config, db, repo};

// ============================================================================
// Configuration
// ============================================================================

/// JIRA authentication configuration
pub const AUTH: AuthConfig = AuthConfig {
    keyring_service: "jira",
    env_var: "JIRA_API_TOKEN",
    cli_command: None,
    display_name: "Jira",
    link_command: "isq link jira",
};

/// Default [on_start] config for JIRA repos
pub const DEFAULT_ON_START_TOML: &str = "transition = \"In Progress\"\nassign_self = true\n";

// OAuth configuration (pub(super) for use in oauth.rs)
pub(super) const JIRA_CLIENT_ID: &str = "VG2jV3YlB3mSWdHcLRZJ8kawl6BFWki8";
pub(super) const JIRA_AUTH_URL: &str = "https://auth.atlassian.com/authorize";
pub(super) const JIRA_RESOURCES_URL: &str = "https://api.atlassian.com/oauth/token/accessible-resources";
pub(super) const REDIRECT_PORT: u16 = 19285;

// OAuth proxy service (handles token exchange with client_secret)
pub(super) const SERVICE_URL: &str = "https://isq-jira-oauth.fly.dev";
pub(super) const REDIRECT_URI: &str = "https://isq-jira-oauth.fly.dev/callback";

// OAuth scopes
pub(super) const JIRA_SCOPES: &str = "read:jira-work write:jira-work read:jira-user manage:jira-project offline_access";

// ============================================================================
// Helper Functions
// ============================================================================

/// Truncate a string to max length with ellipsis (UTF-8 safe)
pub(super) fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        // For very small max_len, just take first chars without ellipsis
        s.chars().take(max_len).collect()
    } else {
        let truncate_at = max_len - 3;
        let truncated: String = s.chars().take(truncate_at).collect();
        format!("{}...", truncated)
    }
}

/// Parse JIRA API error response and return a helpful error message
pub(super) fn parse_jira_error(status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    // Try to parse as JIRA error JSON: {"errorMessages":[], "errors":{"field":"message"}}
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let mut messages = Vec::new();

        // Collect error messages
        if let Some(error_messages) = json.get("errorMessages").and_then(|m| m.as_array()) {
            for msg in error_messages {
                if let Some(s) = msg.as_str() {
                    if !s.is_empty() {
                        messages.push(s.to_string());
                    }
                }
            }
        }

        // Collect field-specific errors with helpful hints
        if let Some(errors) = json.get("errors").and_then(|e| e.as_object()) {
            for (field, msg) in errors {
                if let Some(msg_str) = msg.as_str() {
                    let hint = match field.as_str() {
                        "issuetype" => " (hint: run `isq issue list -o jql=\"project=PROJ\" --json` to see valid issue types, or use -o type=Task)",
                        _ => "",
                    };
                    messages.push(format!("{}: {}{}", field, msg_str, hint));
                }
            }
        }

        if !messages.is_empty() {
            return anyhow!("JIRA error: {}", messages.join("; "));
        }
    }

    // Fallback to raw error
    anyhow!("JIRA API error ({}): {}", status, body)
}

/// Map JIRA priority name to our priority scale.
/// JIRA: Highest, High, Medium, Low, Lowest
/// Ours: 0=urgent, 1=high, 2=medium, 3=low, 4=none
pub(super) fn map_jira_priority(priority_name: Option<&str>) -> u8 {
    match priority_name {
        Some("Highest") => 0,
        Some("High") => 1,
        Some("Medium") => 2,
        Some("Low") => 3,
        Some("Lowest") => 3,
        _ => 4, // unknown/none
    }
}

/// JIRA-specific on_start configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JiraOnStartConfig {
    /// Workflow transition name (e.g., "In Progress", "Start Progress")
    transition: Option<String>,
    /// Assign the issue to yourself
    #[serde(default)]
    assign_self: bool,
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

// ============================================================================
// Credentials
// ============================================================================

/// Get credentials for a specific repo (used by get_forge_for_repo)
pub fn get_credentials_for_repo(_repo_id: &str) -> Result<JiraCredentials> {
    // Try stored credentials first (OAuth flow), then fall back to env var (API token)
    if let Ok(creds) = get_stored_credentials() {
        return Ok(creds);
    }
    if let Ok(creds) = get_credentials_from_env() {
        return Ok(creds);
    }
    Err(anyhow!("No JIRA credentials found. Run 'isq link jira' or set JIRA_API_TOKEN"))
}

// ============================================================================
// Link Flow
// ============================================================================

/// Run the complete JIRA link flow.
/// Handles auth, site selection, project selection, syncs issues, and returns the result.
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::Jira;
    let conn = db::open()?;

    // Try auth in order: env var (for CI) -> keyring -> OAuth
    let creds = if let Ok(env_creds) = get_credentials_from_env() {
        println!("Using JIRA credentials from JIRA_API_TOKEN");
        env_creds
    } else if let Ok(stored_creds) = get_stored_credentials() {
        println!("Using existing JIRA credentials");
        stored_creds
    } else {
        // Run OAuth flow
        let token = oauth_flow().await?;

        // Get accessible sites
        let sites = get_accessible_resources(&token.access_token).await?;
        if sites.is_empty() {
            anyhow::bail!("No JIRA sites accessible with this account");
        }

        // Select site (auto if one, otherwise require -o site=X)
        let site = if sites.len() == 1 {
            println!("Using site: {}", sites[0].name);
            &sites[0]
        } else {
            // Check for site argument
            if let Some(site_name) = args.get("site") {
                sites
                    .iter()
                    .find(|s| {
                        s.name.to_lowercase() == site_name.to_lowercase()
                            || s.url.contains(site_name)
                    })
                    .ok_or_else(|| {
                        let available: Vec<_> = sites.iter().map(|s| &s.name).collect();
                        anyhow!(
                            "Site '{}' not found. Available sites: {:?}",
                            site_name,
                            available
                        )
                    })?
            } else {
                let available: Vec<_> = sites.iter().map(|s| &s.name).collect();
                anyhow::bail!(
                    "Multiple JIRA sites available. Specify one with -o site=<name>.\n\nAvailable sites: {:?}",
                    available
                );
            }
        };

        let expires_at = token
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs as i64);

        let creds = JiraCredentials {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            auth_mode: JiraAuthMode::OAuth { cloud_id: site.id.clone() },
            site_url: site.url.clone(),
            expires_at,
        };

        // Store credentials
        store_credentials(&creds)?;

        creds
    };

    let client = JiraClient::new(creds.clone());

    // List available projects
    let projects = client.list_projects().await?;
    if projects.is_empty() {
        anyhow::bail!("No projects found in this JIRA site");
    }

    // Handle -o list-projects flag
    if args.has_flag("list-projects") {
        println!("Available projects:");
        for project in &projects {
            println!("  {} - {}", project.key, project.name);
        }
        return Err(anyhow!("-o list-projects: showing available projects"));
    }

    // Resolve project from -o project=X argument or auto-select if only one
    let project = if let Some(project_query) = args.get("project") {
        let query_lower = project_query.to_lowercase();
        projects
            .iter()
            .find(|p| p.key.to_lowercase() == query_lower || p.name.to_lowercase() == query_lower)
            .ok_or_else(|| {
                let available: Vec<_> = projects
                    .iter()
                    .map(|p| format!("{} ({})", p.key, p.name))
                    .collect();
                anyhow!(
                    "Project '{}' not found.\n\nAvailable projects:\n  {}",
                    project_query,
                    available.join("\n  ")
                )
            })?
    } else if projects.len() == 1 {
        println!("Using project: {} ({})", projects[0].key, projects[0].name);
        &projects[0]
    } else {
        let available: Vec<_> = projects
            .iter()
            .map(|p| format!("{} ({})", p.key, p.name))
            .collect();
        anyhow::bail!(
            "Multiple projects available. Specify one with -o project=<key>.\n\nAvailable projects:\n  {}\n\nExample: isq link jira -o project=\"{}\"",
            available.join("\n  "),
            projects[0].key
        );
    };

    // Check write permissions
    if !client.check_write_permission(&project.key).await? {
        anyhow::bail!(
            "You don't have write access to project {}. isq requires write permissions to function properly.",
            project.key
        );
    }

    // Get current user for display
    let user = client.get_current_user().await?;
    let display_name = user.display_name.unwrap_or_else(|| user.account_id.clone());

    // Create repo identifier: site/project_key
    let site_host = creds
        .site_url
        .replace("https://", "")
        .replace("http://", "");
    let forge_repo = format!("{}/{}", site_host, project.key);

    // Create pseudo-repo for syncing (JIRA uses site_host as owner, project_key as name)
    let pseudo_repo = repo::Repo {
        owner: site_host.clone(),
        name: project.key.clone(),
    };

    // Sync issues
    println!("Syncing issues from {}...", project.key);
    let issues = client.list_issues_internal(&pseudo_repo, None).await?;

    // Save to database
    let full_display_name = format!("{} ({})", project.name, display_name);
    db::set_repo_link(&conn, repo_path, forge_type.as_str(), &forge_repo, Some(&full_display_name), Some(&user.account_id), Some(&display_name))?;
    db::save_issues(&conn, &forge_repo, &issues.items, true, true)?;
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

    println!("✓ Synced {} issues", issues.items.len());

    // Sync goals
    let goals = client.list_goals(&pseudo_repo).await?;
    db::save_goals(&conn, &forge_repo, &goals)?;
    if !goals.is_empty() {
        println!("✓ Synced {} versions", goals.len());
    }

    Ok(LinkResult {
        display_name: full_display_name,
    })
}

// ============================================================================
// Forge Trait Implementation
// ============================================================================

#[async_trait]
impl Forge for JiraClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, None).await
    }

    async fn list_issues_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, Some(since)).await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        // Get issue type from opts, or default to "Task"
        let issue_type = req.opts.get("type").map(|s| s.as_str()).unwrap_or("Task");
        self.create_issue(repo, &req.title, req.body.as_deref(), &req.labels, issue_type).await
    }

    async fn create_comment(&self, _repo: &Repo, issue_id: &str, body: &str) -> Result<()> {
        let path = format!("/issue/{}/comment", issue_id);

        let comment_body = serde_json::json!({
            "body": adf::markdown_to_adf(body)
        });

        self.post_no_response(&path, &comment_body).await
    }

    async fn close_issue(&self, _repo: &Repo, issue_id: &str) -> Result<()> {
        // Get available transitions
        let path = format!("/issue/{}/transitions", issue_id);
        let response: types::JiraTransitionsResponse = self.get(&path).await?;

        // Find a "Done" transition
        let done_transition = response
            .transitions
            .iter()
            .find(|t| {
                let name_lower = t.name.to_lowercase();
                name_lower == "done" || name_lower.contains("done") || name_lower.contains("close")
            })
            .ok_or_else(|| anyhow!("No 'Done' transition available for this issue"))?;

        let body = serde_json::json!({
            "transition": { "id": done_transition.id }
        });

        self.post_no_response(&path, &body).await
    }

    async fn reopen_issue(&self, _repo: &Repo, issue_id: &str) -> Result<()> {
        // Get available transitions
        let path = format!("/issue/{}/transitions", issue_id);
        let response: types::JiraTransitionsResponse = self.get(&path).await?;

        // Find a "To Do" or "Reopen" transition
        let reopen_transition = response
            .transitions
            .iter()
            .find(|t| {
                let name_lower = t.name.to_lowercase();
                name_lower == "to do"
                    || name_lower.contains("reopen")
                    || name_lower.contains("backlog")
            })
            .ok_or_else(|| anyhow!("No 'Reopen' transition available for this issue"))?;

        let body = serde_json::json!({
            "transition": { "id": reopen_transition.id }
        });

        self.post_no_response(&path, &body).await
    }

    async fn add_label(&self, _repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let path = format!("/issue/{}", issue_id);

        let body = serde_json::json!({
            "update": {
                "labels": [{ "add": label }]
            }
        });

        self.put(&path, &body).await
    }

    async fn remove_label(&self, _repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let path = format!("/issue/{}", issue_id);

        let body = serde_json::json!({
            "update": {
                "labels": [{ "remove": label }]
            }
        });

        self.put(&path, &body).await
    }

    async fn assign_issue(&self, _repo: &Repo, issue_id: &str, assignee: &str) -> Result<()> {
        let path = format!("/issue/{}/assignee", issue_id);

        // Handle unassign case
        let body = if assignee.is_empty() || assignee == "null" {
            serde_json::json!({ "accountId": null })
        } else {
            // assignee should be an account ID
            serde_json::json!({ "accountId": assignee })
        };

        self.put(&path, &body).await
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<db::Comment>> {
        self.list_all_comments_internal(repo, None).await
    }

    async fn list_comments_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<FetchResult<db::Comment>> {
        self.list_all_comments_internal(repo, Some(since)).await
    }

    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        self.list_goals(repo).await
    }

    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal> {
        let project_key = &repo.name;

        // First, get project ID from key
        let project_path = format!("/project/{}", project_key);
        let project: types::JiraProject = self.get(&project_path).await?;

        let project_id: i64 = project.id.parse().map_err(|_| {
            anyhow!("Invalid project ID '{}' - expected numeric value", project.id)
        })?;

        let body = serde_json::json!({
            "name": req.name,
            "description": req.description,
            "releaseDate": req.target_date,
            "projectId": project_id
        });

        let version: types::JiraVersion = self.post("/version", &body).await?;

        Ok(Goal {
            id: version.id,
            name: version.name,
            description: version.description,
            target_date: version.release_date,
            state: GoalState::Open,
            progress: 0.0,
            open_count: None,
            closed_count: None,
            created_at: String::new(),
            updated_at: String::new(),
            html_url: None,
        })
    }

    async fn close_goal(&self, _repo: &Repo, goal_id: &str) -> Result<()> {
        let path = format!("/version/{}", goal_id);
        let body = serde_json::json!({ "released": true });
        self.put(&path, &body).await
    }

    async fn assign_to_goal(&self, _repo: &Repo, issue_id: &str, goal_id: &str) -> Result<()> {
        let path = format!("/issue/{}", issue_id);

        // Get version name from ID
        let version_path = format!("/version/{}", goal_id);
        let version: types::JiraVersion = self.get(&version_path).await?;

        let body = serde_json::json!({
            "update": {
                "fixVersions": [{ "add": { "name": version.name } }]
            }
        });

        self.put(&path, &body).await
    }

    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        // JIRA doesn't expose rate limit info in a simple endpoint
        Ok(None)
    }

    async fn list_labels(&self, _repo: &Repo) -> Result<Vec<Label>> {
        // JIRA labels are freeform and not stored separately
        Ok(Vec::new())
    }

    async fn create_label(
        &self,
        _repo: &Repo,
        name: &str,
        _color: Option<&str>,
        _description: Option<&str>,
    ) -> Result<Label> {
        // JIRA labels are created implicitly when added to issues
        Ok(Label::name_only(name.to_string()))
    }

    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        username: Option<&str>,
    ) -> Result<()> {
        let on_start: JiraOnStartConfig = config.clone().try_into()?;

        // Handle transition
        if let Some(transition_name) = &on_start.transition {
            let path = format!("/issue/{}/transitions", issue_id);
            let response: types::JiraTransitionsResponse = self.get(&path).await?;

            let transition = response
                .transitions
                .iter()
                .find(|t| t.name.to_lowercase() == transition_name.to_lowercase())
                .ok_or_else(|| {
                    let available: Vec<_> = response.transitions.iter().map(|t| &t.name).collect();
                    anyhow!(
                        "Transition '{}' not available. Available transitions: {:?}",
                        transition_name,
                        available
                    )
                })?;

            let body = serde_json::json!({
                "transition": { "id": transition.id }
            });

            self.post_no_response(&path, &body).await?;
        }

        // Handle assign_self
        if on_start.assign_self {
            if let Some(account_id) = username {
                Forge::assign_issue(self, repo, issue_id, account_id).await?;
            }
        }

        Ok(())
    }

    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()> {
        let _: JiraOnStartConfig = config.clone().try_into().context(
            "Invalid [on_start] config for JIRA. Expected: transition = \"In Progress\", assign_self = true",
        )?;
        Ok(())
    }

    async fn handle_command(&self, command: &str, _args: &[String]) -> Result<()> {
        match command {
            "list-fields" => self.list_fields().await,
            _ => Err(anyhow!("Unknown command: {}. Available commands: list-fields", command)),
        }
    }

    async fn query_issues_with_opts(
        &self,
        repo: &Repo,
        opts: &std::collections::HashMap<String, String>,
    ) -> Result<Option<Vec<Issue>>> {
        let jql_opt = opts.get("jql");
        let type_opt = opts.get("type");

        // If no JIRA-specific options, use cache
        if jql_opt.is_none() && type_opt.is_none() {
            return Ok(None);
        }

        let project_key = &repo.name;

        // Build JQL from options
        let mut conditions = vec![format!("project = {}", project_key)];

        if let Some(jql) = jql_opt {
            conditions.push(format!("({})", jql));
        }

        if let Some(issue_type) = type_opt {
            conditions.push(format!("issuetype = \"{}\"", issue_type));
        }

        let full_jql = conditions.join(" AND ");

        let mut all_issues = Vec::new();
        let mut next_page_token: Option<String> = None;

        loop {
            let body = serde_json::json!({
                "jql": full_jql,
                "maxResults": 100,
                "fields": ["*all"],
                "nextPageToken": next_page_token
            });

            let response: types::JiraSearchResponse = self.post("/search/jql", &body).await?;

            for jira_issue in &response.issues {
                all_issues.push(self.convert_issue(jira_issue));
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(Some(all_issues))
    }
}
