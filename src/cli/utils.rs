//! Shared CLI utilities

use std::io::{self, IsTerminal, Read};

use anyhow::Result;
use serde::Serialize;

use crate::repo::Repo;
use crate::service;

/// JSON response for write operations
#[derive(Serialize)]
pub struct WriteResult {
    pub success: bool,
    pub queued: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    pub message: String,
    pub elapsed_ms: u64,
}

/// Check if an error is a network/connectivity error (offline)
pub fn is_offline_error(err: &anyhow::Error) -> bool {
    let err_str = err.to_string().to_lowercase();
    err_str.contains("connection refused")
        || err_str.contains("network is unreachable")
        || err_str.contains("no route to host")
        || err_str.contains("dns error")
        || err_str.contains("connection reset")
        || err_str.contains("timed out")
        || err_str.contains("could not resolve")
}

/// Normalize an issue ID to the format stored in the database.
///
/// For forges that use prefixed IDs (Linear, JIRA), this ensures the prefix is present.
/// - Linear: "413" -> "WRK-413" (prefix from forge_repo first component)
/// - JIRA: "123" -> "DEV-123" (prefix from forge_repo last component)
/// - GitHub: "123" -> "123" (no prefix needed)
///
/// If the ID already has a prefix, it's returned as-is (uppercased for consistency).
pub fn normalize_issue_id(id: &str, forge_type: &str, forge_repo: &str) -> String {
    // If already has a prefix, return as-is (uppercase the prefix for consistency)
    if id.contains('-') {
        let parts: Vec<&str> = id.splitn(2, '-').collect();
        if parts.len() == 2 {
            return format!("{}-{}", parts[0].to_uppercase(), parts[1]);
        }
    }

    // Add prefix based on forge type
    match forge_type {
        "linear" => {
            // forge_repo is "TEAM_KEY/workspace_uuid", prefix is the team key
            let prefix = forge_repo.split('/').next().unwrap_or("");
            format!("{}-{}", prefix, id)
        }
        "jira" => {
            // forge_repo is "site/PROJECT_KEY", prefix is the project key
            let prefix = forge_repo.split('/').next_back().unwrap_or("");
            format!("{}-{}", prefix, id)
        }
        _ => {
            // GitHub and others: no prefix needed
            id.to_string()
        }
    }
}

/// Ensure the system service is installed and running
pub fn ensure_service_running() -> Result<()> {
    let status = service::status()?;

    if !status.installed {
        println!("Installing system service...");
        service::install()?;
        println!("✓ System service installed");
    } else if !status.running {
        service::start()?;
        println!("✓ System service started");
    } else if let Some(pid) = status.pid {
        println!("System service running (PID {})", pid);
    }

    Ok(())
}

/// Read content from stdin if it's being piped (not a TTY).
/// Returns None if stdin is a TTY (interactive mode).
/// Returns Some(content) if stdin has piped content.
pub fn read_stdin_if_piped() -> Result<Option<String>> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    let mut content = String::new();
    stdin.lock().read_to_string(&mut content)?;
    let content = content.trim().to_string();
    if content.is_empty() {
        return Ok(None);
    }
    Ok(Some(content))
}

/// Parse a forge_repo string (e.g., "owner/name") into a Repo struct.
pub fn parse_forge_repo(forge_repo: &str) -> Result<Repo> {
    let parts: Vec<&str> = forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", forge_repo);
    }
    Ok(Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

/// Validate that a JIRA issue key prefix matches the linked project.
/// Only validates if the issue_id contains a prefix (has a hyphen).
pub fn validate_jira_issue_prefix(
    issue_id: &str,
    project_key: &str,
    forge_type: &str,
) -> Result<()> {
    if forge_type != "jira" {
        return Ok(());
    }
    if issue_id.contains('-') {
        let prefix = issue_id.split('-').next().unwrap_or("");
        if !prefix.eq_ignore_ascii_case(project_key) {
            anyhow::bail!(
                "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                issue_id,
                prefix,
                project_key
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_issue_id_linear_number_only() {
        assert_eq!(
            normalize_issue_id("413", "linear", "WRK/5328bff7-5748-4e00-a582-79c23d647aca"),
            "WRK-413"
        );
    }

    #[test]
    fn test_normalize_issue_id_linear_with_prefix() {
        assert_eq!(
            normalize_issue_id(
                "WRK-413",
                "linear",
                "WRK/5328bff7-5748-4e00-a582-79c23d647aca"
            ),
            "WRK-413"
        );
    }

    #[test]
    fn test_normalize_issue_id_linear_lowercase_prefix() {
        assert_eq!(
            normalize_issue_id(
                "wrk-413",
                "linear",
                "WRK/5328bff7-5748-4e00-a582-79c23d647aca"
            ),
            "WRK-413"
        );
    }

    #[test]
    fn test_normalize_issue_id_jira_number_only() {
        assert_eq!(
            normalize_issue_id("123", "jira", "site.atlassian.net/DEV"),
            "DEV-123"
        );
    }

    #[test]
    fn test_normalize_issue_id_jira_with_prefix() {
        assert_eq!(
            normalize_issue_id("DEV-123", "jira", "site.atlassian.net/DEV"),
            "DEV-123"
        );
    }

    #[test]
    fn test_normalize_issue_id_github_unchanged() {
        assert_eq!(normalize_issue_id("123", "github", "owner/repo"), "123");
    }
}
