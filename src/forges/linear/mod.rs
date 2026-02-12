//! Linear issue tracker integration
//!
//! This module provides Linear API client functionality including:
//! - OAuth PKCE authentication flow
//! - Team and issue management
//! - Project (goal) operations
//! - Comment syncing

mod auth;
mod client;
mod comments;
mod forge_impl;
mod issues;
mod link;
mod oauth;
mod projects;
mod queries;
mod states;
mod types;
mod updates;
mod users;

use anyhow::anyhow;
use serde::Deserialize;

pub use client::LinearClient;
pub use link::link;
#[allow(unused_imports)]
pub use oauth::{TokenResponse, oauth_flow, refresh_token};
#[allow(unused_imports)]
pub use types::{LinearOrganization, LinearProject, LinearTeam};

use super::{AuthConfig, ForgeType};

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

/// Map our priority scale to Linear priority.
/// Ours:   0=urgent, 1=high, 2=medium, 3=low, 4=none
/// Linear: 0=none, 1=urgent, 2=high, 3=normal, 4=low
pub(super) fn map_to_linear_priority(priority: u8) -> u8 {
    match priority {
        0 => 1, // urgent → urgent
        1 => 2, // high → high
        2 => 3, // medium → normal
        3 => 4, // low → low
        4 => 0, // none → no priority
        _ => 0, // unknown → no priority
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
    fn test_map_to_linear_priority() {
        assert_eq!(map_to_linear_priority(0), 1);
        assert_eq!(map_to_linear_priority(1), 2);
        assert_eq!(map_to_linear_priority(2), 3);
        assert_eq!(map_to_linear_priority(3), 4);
        assert_eq!(map_to_linear_priority(4), 0);
        assert_eq!(map_to_linear_priority(255), 0);
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
