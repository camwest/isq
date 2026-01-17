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

    // Update the install receipt if it exists
    if install::read_receipt()?.is_some() {
        install::update_receipt_version(&result.new_version)?;
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

    Ok(UpdateResult {
        previous_version: current_version.to_string(),
        new_version: status.version().to_string(),
    })
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
}
