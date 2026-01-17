//! Shared CLI utilities

use std::io::{self, IsTerminal, Read};

use anyhow::Result;
use serde::Serialize;

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

/// Parse an issue ID string to extract the numeric portion.
/// Supports both formats: "123" or "DEV-123" (returns 123 in both cases).
/// If expected_prefix is provided and the ID contains a prefix, validates they match.
pub fn parse_issue_number(id: &str, expected_prefix: Option<&str>) -> Result<u64> {
    // Check if ID contains a project prefix (e.g., "DEV-123")
    if id.contains('-') {
        let parts: Vec<&str> = id.splitn(2, '-').collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let num_str = parts[1];

            // Validate prefix if expected
            if let Some(expected) = expected_prefix {
                if !prefix.eq_ignore_ascii_case(expected) {
                    anyhow::bail!(
                        "Issue '{}' belongs to project '{}', but you're linked to '{}'. \
                         Cross-project operations will be supported in a future release (see issue #74).",
                        id, prefix, expected
                    );
                }
            }

            return num_str.parse::<u64>().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid issue ID: '{}'. Expected a number or key like DEV-123",
                    id
                )
            });
        }
    }

    // Plain number
    id.parse::<u64>().map_err(|_| {
        anyhow::anyhow!(
            "Invalid issue ID: '{}'. Expected a number or key like DEV-123",
            id
        )
    })
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
