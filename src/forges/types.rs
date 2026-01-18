//! Core data types for forge abstraction.

use std::panic;

use reqwest::Client;
use serde::{Deserialize, Serialize};

// ============================================================================
// HTTP Client
// ============================================================================

/// Create a reqwest Client, falling back to no_proxy if system proxy detection panics.
///
/// On macOS, reqwest tries to read system proxy settings via the system-configuration
/// crate. In sandboxed environments (e.g., Claude Code), this can panic because
/// access to macOS system configuration is blocked. This function catches that panic
/// and falls back to a client without system proxy support.
///
/// On Linux, proxy detection reads environment variables and never panics.
#[allow(clippy::redundant_closure)]
pub fn create_http_client() -> Client {
    // Try to create a client with system proxy support
    // Note: We need the closure here because Client::new isn't UnwindSafe
    let result = panic::catch_unwind(|| Client::new());

    match result {
        Ok(client) => client,
        Err(_) => {
            // System proxy detection panicked (likely sandboxed macOS)
            // Fall back to a client without system proxy support
            Client::builder()
                .no_proxy()
                .build()
                .expect("Failed to create HTTP client without proxy")
        }
    }
}

// ============================================================================
// Fetch Result
// ============================================================================

/// Result of a fetch operation with completeness tracking
#[derive(Debug, Clone)]
pub struct FetchResult<T> {
    pub items: Vec<T>,
    /// True only if ALL pages succeeded; false if any partial failure
    pub is_complete: bool,
}

impl<T> FetchResult<T> {
    /// Create a complete fetch result
    pub fn complete(items: Vec<T>) -> Self {
        Self {
            items,
            is_complete: true,
        }
    }

    /// Create an incomplete fetch result (partial failure)
    pub fn incomplete(items: Vec<T>) -> Self {
        Self {
            items,
            is_complete: false,
        }
    }
}

// ============================================================================
// Issue Types
// ============================================================================

/// A label with optional color
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    /// Hex color without #, e.g., "fc2929"
    pub color: Option<String>,
}

impl Label {
    pub fn new(name: String, color: Option<String>) -> Self {
        Self { name, color }
    }

    /// Create a label with just a name (no color)
    pub fn name_only(name: String) -> Self {
        Self { name, color: None }
    }
}

/// Forge-agnostic issue representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Unique issue identifier - format varies by forge:
    /// - GitHub: "123" (issue number as string)
    /// - Linear: "DEV-123" (identifier)
    /// - JIRA: "PROJ-123" (key)
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub author: String,
    pub labels: Vec<Label>,
    /// Usernames of assignees (GitHub: login, Linear: displayName)
    pub assignees: Vec<String>,
    /// Priority level: 0=urgent, 1=high, 2=medium, 3=low, 4=none
    /// Linear: native priority field. GitHub: mapped from labels via config.
    pub priority: u8,
    /// Label name that was used to determine priority (GitHub only, for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_label: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub url: Option<String>,
    /// Goal name (GitHub: milestone title, Linear: project name)
    pub milestone: Option<String>,
}

// ============================================================================
// Goal Types
// ============================================================================

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

// ============================================================================
// Request Types
// ============================================================================

/// Request to create an issue
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<String>,
    pub goal_id: Option<String>,
    /// Forge-specific options (e.g., type=Bug for JIRA)
    pub opts: std::collections::HashMap<String, String>,
}

/// Parse forge-specific options from CLI -o key=value arguments
pub fn parse_opts(opts: &[String]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for opt in opts {
        if let Some((key, value)) = opt.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

/// Rate limit status from a forge
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// Total requests allowed per hour
    pub limit: u32,
    /// Requests remaining this hour
    pub remaining: u32,
    /// Unix timestamp when the limit resets
    pub reset_at: i64,
}
