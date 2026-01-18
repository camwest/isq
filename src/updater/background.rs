//! Background update checking and download

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use chrono::Utc;

use crate::install::{self, InstallMethod};

use super::check::{UpdateInfo, check_for_updates};
use super::staged::{staged_update_dir, staged_update_path};

/// Minimum hours between background update checks
const CHECK_INTERVAL_HOURS: i64 = 24;

/// Spawn a background task to check for updates (non-blocking).
///
/// This function returns immediately. The update check runs in a separate
/// tokio task and will not block CLI startup.
///
/// Only checks if:
/// - Install method is Standalone
/// - At least 24 hours since last check
pub fn maybe_check_for_updates_background() {
    tokio::spawn(async {
        if let Err(_e) = background_update_check().await {
            // Silently ignore errors - this is best-effort
            // Debug logging could be added here if needed
        }
    });
}

/// Perform the actual background update check.
///
/// This function:
/// 1. Checks install method (skips if not Standalone)
/// 2. Checks cooldown (skips if checked within 24h)
/// 3. Checks for available updates
/// 4. Downloads update to staging if available
/// 5. Updates receipt with staged version and last check time
async fn background_update_check() -> Result<()> {
    // 1. Check install method - only Standalone gets automatic background updates
    // Unknown installs can still manually run `isq update install`, but we don't
    // automatically download updates for them since we can't be sure it's safe
    let install_method = install::detect_install_method();
    if install_method != InstallMethod::Standalone {
        return Ok(());
    }

    // 2. Check cooldown
    if !should_check_for_updates()? {
        return Ok(()); // Checked recently, skip
    }

    // 3. Check for updates
    let update_info = match check_for_updates().await? {
        Some(info) => info,
        None => {
            // No update available - still update check time
            let _ = install::update_last_check_time();
            return Ok(());
        }
    };

    // 4. Download to staging
    download_to_staging(&update_info).await?;

    // 5. Update receipt with staged version and check time
    install::update_staged_version(Some(&update_info.latest_version))?;
    install::update_last_check_time()?;

    Ok(())
}

/// Check if we should perform an update check based on cooldown.
///
/// Returns true if enough time has passed since last check (or never checked).
/// Returns false if no receipt exists (non-standalone install).
fn should_check_for_updates() -> Result<bool> {
    let Some(receipt) = install::read_receipt()? else {
        return Ok(false); // No receipt means non-standalone install
    };

    let Some(last_check) = receipt.last_update_check else {
        return Ok(true); // Never checked
    };

    let elapsed = Utc::now() - last_check;
    Ok(elapsed.num_hours() >= CHECK_INTERVAL_HOURS)
}

/// Download the update binary to the staging directory.
async fn download_to_staging(info: &UpdateInfo) -> Result<()> {
    // Check for valid download URL (might be empty if no asset matches platform)
    if info.download_url.is_empty() {
        return Err(anyhow!("No download available for this platform"));
    }

    let staged_dir = staged_update_dir()?;
    let staged_path = staged_update_path()?;

    // Create staging directory
    std::fs::create_dir_all(&staged_dir)?;

    // Download using self_update's download mechanism
    tokio::task::spawn_blocking({
        let download_url = info.download_url.clone();
        move || download_to_staging_blocking(&download_url, &staged_path)
    })
    .await??;

    Ok(())
}

/// Blocking download to staging location.
fn download_to_staging_blocking(download_url: &str, staged_path: &PathBuf) -> Result<()> {
    // Create a temp file for the download
    let tmp_dir = tempfile::tempdir()?;
    let tmp_archive_path = tmp_dir.path().join("isq-release.tar.gz");

    // Download the release asset
    let mut tmp_file = std::fs::File::create(&tmp_archive_path)?;
    self_update::Download::from_url(download_url)
        .set_header(reqwest::header::ACCEPT, "application/octet-stream".parse()?)
        .download_to(&mut tmp_file)?;
    drop(tmp_file); // Ensure file is closed before extracting

    // Extract the binary from the archive
    // self_update::Extract handles tar.gz automatically
    let staged_dir = staged_path
        .parent()
        .ok_or_else(|| anyhow!("Invalid staged path"))?;
    self_update::Extract::from_source(&tmp_archive_path).extract_file(staged_dir, "isq")?;

    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(staged_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(staged_path, perms)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_interval_hours() {
        // Verify the constant is set correctly
        assert_eq!(CHECK_INTERVAL_HOURS, 24);
    }
}
