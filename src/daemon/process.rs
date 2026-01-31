//! Daemon lifecycle management - PID files, locking, and daemon info.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::PathBuf;

/// Information about the running daemon, stored in the PID file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub version: String,
    pub started_at: DateTime<Utc>,
}

/// Get the daemon PID file path
pub fn pid_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;

    Ok(cache_dir.join("daemon.pid"))
}

/// Get the daemon lock file path
fn lock_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;

    Ok(cache_dir.join("daemon.lock"))
}

/// Acquire exclusive lock on the daemon lock file.
/// Returns the File handle which must be kept alive for the lock to remain held.
/// Returns error if another instance already holds the lock.
pub fn acquire_lock() -> Result<File> {
    use std::os::unix::io::AsRawFd;

    let path = lock_path()?;
    let file = File::create(&path)?;

    // Try exclusive lock (non-blocking)
    let fd = file.as_raw_fd();
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

    if result != 0 {
        anyhow::bail!("Another daemon instance is already running");
    }

    Ok(file)
}

/// Write daemon info to the PID file in JSON format.
pub fn write_daemon_info(info: &DaemonInfo) -> Result<()> {
    let pid_file = pid_path()?;
    let content = serde_json::to_string_pretty(info)?;
    fs::write(&pid_file, content)?;
    Ok(())
}

/// Read daemon info from the PID file.
///
/// Returns None if file doesn't exist or is invalid JSON.
pub fn read_daemon_info() -> Result<Option<DaemonInfo>> {
    let pid_file = pid_path()?;

    if !pid_file.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&pid_file)?;
    Ok(serde_json::from_str(&content).ok())
}
