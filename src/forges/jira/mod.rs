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
mod comments;
mod fields;
mod forge_impl;
mod goals;
mod issues;
mod link;
mod oauth;
mod types;

use anyhow::{Result, anyhow};

pub use client::JiraClient;
pub use link::link;
#[allow(unused_imports)]
pub use oauth::{AccessibleResource, TokenResponse, refresh_token};
#[allow(unused_imports)]
pub use oauth::{
    JiraAuthMode, JiraCredentials, get_accessible_resources, get_credentials_from_env,
    get_stored_credentials, oauth_flow, store_credentials,
};
#[allow(unused_imports)]
pub use types::{JiraProject, JiraUser, JiraVersion};

use super::AuthConfig;

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

/// Default [on_cleanup] config for JIRA repos
/// Commented out by default since moving issues back may not be desired
pub const DEFAULT_ON_CLEANUP_TOML: &str =
    "# transition = \"To Do\"  # Optional: move issue back to backlog\n";

// OAuth configuration (pub(super) for use in oauth.rs)
pub(super) const JIRA_CLIENT_ID: &str = "VG2jV3YlB3mSWdHcLRZJ8kawl6BFWki8";
pub(super) const JIRA_AUTH_URL: &str = "https://auth.atlassian.com/authorize";
pub(super) const JIRA_RESOURCES_URL: &str =
    "https://api.atlassian.com/oauth/token/accessible-resources";
pub(super) const REDIRECT_PORT: u16 = 19285;

// OAuth proxy service (handles token exchange with client_secret)
pub(super) const SERVICE_URL: &str = "https://isq-jira-oauth.fly.dev";
pub(super) const REDIRECT_URI: &str = "https://isq-jira-oauth.fly.dev/callback";

// OAuth scopes
pub(super) const JIRA_SCOPES: &str =
    "read:jira-work write:jira-work read:jira-user manage:jira-project offline_access";

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
                if let Some(s) = msg.as_str()
                    && !s.is_empty()
                {
                    messages.push(s.to_string());
                }
            }
        }

        // Collect field-specific errors with helpful hints
        if let Some(errors) = json.get("errors").and_then(|e| e.as_object()) {
            for (field, msg) in errors {
                if let Some(msg_str) = msg.as_str() {
                    let hint = match field.as_str() {
                        "issuetype" => {
                            " (hint: run `isq issue list -o jql=\"project=PROJ\" --json` to see valid issue types, or use -o type=Task)"
                        }
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
    Err(anyhow!(
        "No JIRA credentials found. Run 'isq link jira' or set JIRA_API_TOKEN"
    ))
}
