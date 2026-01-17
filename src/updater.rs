//! Update checking logic using GitHub Releases API

use anyhow::{anyhow, Result};
use self_update::backends::github::ReleaseList;
use semver::Version;
use serde::Serialize;

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
