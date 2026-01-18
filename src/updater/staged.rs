//! Staged update system for safe binary replacement

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::install;
use crate::user_config;

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
pub(crate) fn remove_staged_files() {
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
