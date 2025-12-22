use anyhow::{anyhow, Result};
use std::process::Command;

use crate::db;

/// Get GitHub token from gh CLI
pub fn get_gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|_| anyhow!("gh CLI not found. Install it from https://cli.github.com"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh CLI not authenticated. Run `gh auth login` first.\n{}",
            stderr.trim()
        ));
    }

    let token = String::from_utf8(output.stdout)?.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("gh CLI returned empty token"));
    }

    Ok(token)
}

/// Get GitHub token with fallback: gh CLI → stored credentials → env var
pub fn get_github_token() -> Result<String> {
    // Try gh CLI first (fastest, most common)
    if let Ok(token) = get_gh_token() {
        return Ok(token);
    }

    // Try stored credentials (from OAuth)
    if let Ok(conn) = db::open() {
        if let Ok(Some(cred)) = db::get_credential(&conn, "github") {
            return Ok(cred.access_token);
        }
    }

    // Try environment variable
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Ok(token);
    }

    // No token available
    Err(anyhow!(
        "GitHub not authenticated.\n\n\
        Option 1: Install gh CLI and run: gh auth login\n\
        Option 2: Run: isq link github (browser OAuth)"
    ))
}

/// Check if GitHub auth is available (without triggering OAuth)
pub fn has_github_credentials() -> bool {
    // Check gh CLI
    if get_gh_token().is_ok() {
        return true;
    }

    // Check stored credentials
    if let Ok(conn) = db::open() {
        if let Ok(Some(_)) = db::get_credential(&conn, "github") {
            return true;
        }
    }

    // Check env var
    std::env::var("GITHUB_TOKEN").is_ok()
}

/// Get Linear token from stored credentials or environment variable
pub fn get_linear_token() -> Result<String> {
    // First check stored credentials
    if let Ok(conn) = db::open() {
        if let Ok(Some(cred)) = db::get_credential(&conn, "linear") {
            return Ok(cred.access_token);
        }
    }

    // Fall back to environment variable
    std::env::var("LINEAR_API_KEY").map_err(|_| {
        anyhow!(
            "Linear not authenticated.\n\n\
            Run: isq link linear"
        )
    })
}

/// Check if Linear has stored credentials (not just env var)
pub fn has_linear_credentials() -> bool {
    if let Ok(conn) = db::open() {
        if let Ok(Some(_)) = db::get_credential(&conn, "linear") {
            return true;
        }
    }
    std::env::var("LINEAR_API_KEY").is_ok()
}
