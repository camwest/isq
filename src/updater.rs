//! Update checking and application logic using GitHub Releases API

use anyhow::{anyhow, Result};
use self_update::backends::github::ReleaseList;
use semver::Version;
use serde::Serialize;

use crate::install::{self, InstallMethod};

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    pub published_at: String,
}

/// Check for available updates from GitHub Releases
///
/// Returns Some(UpdateInfo) if a newer version is available, None if up to date.
/// Skips prereleases (versions containing '-').
///
/// This function runs the blocking HTTP call in a separate thread to avoid
/// blocking the async runtime.
pub async fn check_for_updates() -> Result<Option<UpdateInfo>> {
    // self_update uses blocking HTTP internally, so we need to run it
    // in a blocking thread to avoid issues with the async runtime
    tokio::task::spawn_blocking(check_for_updates_blocking)
        .await?
}

fn check_for_updates_blocking() -> Result<Option<UpdateInfo>> {
    let current = env!("CARGO_PKG_VERSION");

    let releases = ReleaseList::configure()
        .repo_owner("camwest")
        .repo_name("isq")
        .build()?
        .fetch()?;

    // Filter to stable releases only (skip prereleases)
    let latest = releases
        .iter()
        .find(|r| !r.version.contains('-'))
        .ok_or_else(|| anyhow!("No releases found"))?;

    let current_v = Version::parse(current)?;
    let latest_v = Version::parse(&latest.version)?;

    if latest_v > current_v {
        // Find download URL for current platform
        let target = self_update::get_target();
        let download_url = latest
            .asset_for(target, None)
            .map(|a| a.download_url)
            .unwrap_or_default();

        Ok(Some(UpdateInfo {
            current_version: current.to_string(),
            latest_version: latest.version.clone(),
            download_url,
            release_notes: latest.body.clone(),
            published_at: latest.date.clone(),
        }))
    } else {
        Ok(None)
    }
}

/// Result of a successful update
#[derive(Debug)]
pub struct UpdateResult {
    pub previous_version: String,
    pub new_version: String,
}

/// Apply an available update by downloading and replacing the binary.
///
/// This function:
/// 1. Checks install method (only works for standalone/unknown)
/// 2. Downloads the new version from GitHub Releases
/// 3. Atomically replaces the current binary
/// 4. Updates the install receipt with the new version
///
/// Only works for standalone installations. Returns an error for other install methods.
pub async fn apply_update() -> Result<UpdateResult> {
    let install_method = install::detect_install_method();

    // Self-update only works for standalone installations
    let update_cmd = match install_method {
        InstallMethod::Standalone | InstallMethod::Unknown => None,
        InstallMethod::Homebrew => Some("brew upgrade isq"),
        InstallMethod::Scoop => Some("scoop update isq"),
        InstallMethod::Cargo => Some("cargo install isq"),
    };

    if let Some(cmd) = update_cmd {
        anyhow::bail!(
            "Self-update is only available for standalone installations.\n\
             Run `{}` to update.",
            cmd
        );
    }

    let result = tokio::task::spawn_blocking(apply_update_blocking).await??;

    // Update the install receipt if it exists (non-fatal if it fails)
    if install::read_receipt()?.is_some() {
        if let Err(e) = install::update_receipt_version(&result.new_version) {
            eprintln!("Warning: Failed to update install receipt: {}", e);
        }
    }

    Ok(result)
}

fn apply_update_blocking() -> Result<UpdateResult> {
    let current_version = env!("CARGO_PKG_VERSION");

    let status = self_update::backends::github::Update::configure()
        .repo_owner("camwest")
        .repo_name("isq")
        .bin_name("isq")
        .show_download_progress(true)
        .current_version(current_version)
        .build()?
        .update()?;

    // Handle race condition: version could change between check and apply
    if status.version() == current_version {
        anyhow::bail!("Already on the latest version ({})", current_version);
    }

    Ok(UpdateResult {
        previous_version: current_version.to_string(),
        new_version: status.version().to_string(),
    })
}

/// Parse version from `isq --version` output.
/// Format: "isq 0.1.0" or "isq 0.1.0 (standalone, auto-updates enabled)"
fn parse_version_from_output(output: &str) -> Result<String> {
    let line = output.lines().next().unwrap_or("");
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 && parts[0] == "isq" {
        Ok(parts[1].to_string())
    } else {
        anyhow::bail!("Could not parse version from: {}", output)
    }
}

/// Get version of the binary on disk by running `isq --version`.
///
/// This spawns a subprocess to get the actual compiled-in version of the
/// binary file, which may differ from our running version if the binary
/// was updated while we're running.
///
/// Includes a 5-second timeout to prevent hanging if the subprocess gets stuck.
pub async fn get_binary_version_on_disk() -> Result<String> {
    use std::time::Duration;

    let exe = std::env::current_exe()?;
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(&exe)
            .arg("--version")
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Version check timed out after 5 seconds"))??;

    if !output.status.success() {
        anyhow::bail!(
            "Failed to get binary version: exit code {:?}",
            output.status.code()
        );
    }

    parse_version_from_output(&String::from_utf8_lossy(&output.stdout))
}

/// Check if running version differs from binary on disk.
///
/// Returns true if the disk binary has a different version, indicating
/// the daemon should restart to pick up the new version.
pub async fn is_binary_updated() -> Result<bool> {
    let running = env!("CARGO_PKG_VERSION");
    let on_disk = get_binary_version_on_disk().await?;
    Ok(running != on_disk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        // These tests verify semver comparison logic
        let v1 = Version::parse("0.1.0").unwrap();
        let v2 = Version::parse("0.2.0").unwrap();
        assert!(v2 > v1);

        let v3 = Version::parse("0.2.0-beta").unwrap();
        assert!(v2 > v3); // stable > prerelease

        let v4 = Version::parse("1.0.0").unwrap();
        let v5 = Version::parse("0.99.99").unwrap();
        assert!(v4 > v5);
    }

    #[test]
    fn test_prerelease_detection() {
        assert!("0.2.0-beta".contains('-'));
        assert!("0.2.0-rc.1".contains('-'));
        assert!(!"0.2.0".contains('-'));
        assert!(!"1.0.0".contains('-'));
    }

    #[test]
    fn test_parse_version_from_output() {
        // Basic format
        assert_eq!(parse_version_from_output("isq 0.1.0").unwrap(), "0.1.0");

        // With install method suffix
        assert_eq!(
            parse_version_from_output("isq 0.2.0 (standalone)").unwrap(),
            "0.2.0"
        );

        // With auto-update info
        assert_eq!(
            parse_version_from_output("isq 1.0.0 (standalone, auto-updates enabled)").unwrap(),
            "1.0.0"
        );

        // With trailing newline
        assert_eq!(parse_version_from_output("isq 0.1.0\n").unwrap(), "0.1.0");

        // Multi-line output (homebrew includes note)
        assert_eq!(
            parse_version_from_output(
                "isq 0.1.0 (homebrew)\nNote: Run `brew upgrade isq` to update."
            )
            .unwrap(),
            "0.1.0"
        );
    }

    #[test]
    fn test_parse_version_from_output_errors() {
        // Empty string
        assert!(parse_version_from_output("").is_err());

        // Wrong prefix
        assert!(parse_version_from_output("foo 0.1.0").is_err());

        // No version
        assert!(parse_version_from_output("isq").is_err());

        // Just whitespace
        assert!(parse_version_from_output("   ").is_err());
    }
}
