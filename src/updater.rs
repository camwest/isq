//! Update checking and application logic using GitHub Releases API

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use chrono::Utc;
use self_update::backends::github::ReleaseList;
use semver::Version;
use serde::Serialize;

use crate::install::{self, InstallMethod};
use crate::user_config;

/// Minimum hours between background update checks
const CHECK_INTERVAL_HOURS: i64 = 24;

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
    tokio::task::spawn_blocking(check_for_updates_blocking).await?
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
    if install::read_receipt()?.is_some()
        && let Err(e) = install::update_receipt_version(&result.new_version)
    {
        eprintln!("Warning: Failed to update install receipt: {}", e);
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

// =============================================================================
// Binary Version Detection (for daemon auto-restart)
// =============================================================================

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
        tokio::process::Command::new(&exe).arg("--version").output(),
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
// =============================================================================
// Staged Update System
// =============================================================================

/// Information about a staged update ready to apply
#[derive(Debug)]
pub struct StagedUpdate {
    pub version: String,
    pub path: PathBuf,
}

/// Get the path to the staged update directory
pub fn staged_update_dir() -> Result<PathBuf> {
    Ok(user_config::config_dir()?.join("staged-update"))
}

/// Get the path to the staged update binary
pub fn staged_update_path() -> Result<PathBuf> {
    Ok(staged_update_dir()?.join("isq"))
}

/// Check if there's a valid staged update ready to apply.
///
/// Returns Some(StagedUpdate) if both:
/// - The receipt has a staged_update_version
/// - The staged binary exists on disk
///
/// Cleans up inconsistent state (orphan files or stale receipt entries).
pub fn check_staged_update() -> Result<Option<StagedUpdate>> {
    let staged_path = staged_update_path()?;

    // Get staged version from receipt
    let staged_version = install::read_receipt()?.and_then(|r| r.staged_update_version);

    match (staged_version, staged_path.exists()) {
        (Some(version), true) => Ok(Some(StagedUpdate {
            version,
            path: staged_path,
        })),
        (Some(_), false) => {
            // Receipt says staged but file missing - clear stale receipt entry
            let _ = install::update_staged_version(None);
            Ok(None)
        }
        (None, true) => {
            // Orphan staged file without receipt entry - clean up
            remove_staged_files();
            Ok(None)
        }
        (None, false) => Ok(None),
    }
}

/// Apply a staged update by replacing the current binary.
///
/// This function:
/// 1. Replaces the current binary with the staged one
/// 2. Deletes the staged binary
/// 3. Clears staged_update_version from receipt
/// 4. Updates the version in receipt
pub fn apply_staged_update(staged: &StagedUpdate) -> Result<()> {
    let current_exe = std::env::current_exe()?;

    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        // Copy staged binary over current. Not atomic across filesystems, but
        // the staged binary remains intact if interrupted, so next startup retries.
        fs::copy(&staged.path, &current_exe)?;

        // Ensure executable permissions
        let mut perms = fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&current_exe, perms)?;
    }

    #[cfg(not(unix))]
    {
        // On Windows, rename current to .old, then copy staged to current
        let old_exe = current_exe.with_extension("exe.old");
        let _ = std::fs::remove_file(&old_exe); // Remove any previous .old
        std::fs::rename(&current_exe, &old_exe)?;

        // If copy fails, try to restore the old binary
        if let Err(e) = std::fs::copy(&staged.path, &current_exe) {
            let _ = std::fs::rename(&old_exe, &current_exe);
            return Err(e.into());
        }

        let _ = std::fs::remove_file(&old_exe); // Clean up .old
    }

    // Clean up staged binary and update receipt
    remove_staged_files();
    let _ = install::update_staged_version(None);
    let _ = install::update_receipt_version(&staged.version);

    Ok(())
}

/// Remove staged update files from disk.
fn remove_staged_files() {
    if let Ok(path) = staged_update_path() {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(dir) = staged_update_dir() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Clean up staged update files without applying.
///
/// Called when staged update is invalid or cannot be applied.
pub fn cleanup_staged_update() {
    remove_staged_files();
    let _ = install::update_staged_version(None);
}

/// Re-execute the current binary to run the newly applied version.
///
/// This function does not return on success - it replaces the current process.
pub fn restart_self() -> Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().collect();

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new(&exe);
        if args.len() > 1 {
            cmd.args(&args[1..]);
        }
        // exec replaces this process - never returns on success
        let err = cmd.exec();
        Err(anyhow!("Failed to restart: {}", err))
    }

    #[cfg(not(unix))]
    {
        // On Windows, spawn a new process and exit
        let mut cmd = std::process::Command::new(&exe);
        if args.len() > 1 {
            cmd.args(&args[1..]);
        }
        cmd.spawn()?;
        std::process::exit(0);
    }
}

// =============================================================================
// Background Update Check
// =============================================================================

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

    #[test]
    fn test_check_interval_hours() {
        // Verify the constant is set correctly
        assert_eq!(CHECK_INTERVAL_HOURS, 24);
    }

    #[test]
    fn test_staged_update_path_format() {
        // Verify the staged update path is in the expected location
        if let Ok(path) = staged_update_path() {
            assert!(path.to_string_lossy().contains("staged-update"));
            assert!(path.to_string_lossy().ends_with("isq"));
        }
    }

    #[test]
    fn test_staged_update_dir_format() {
        // Verify the staged update dir is in the expected location
        if let Ok(path) = staged_update_dir() {
            assert!(path.to_string_lossy().contains("staged-update"));
        }
    }
}
