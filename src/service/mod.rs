use anyhow::{Result, anyhow};
use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// Service status information
#[derive(Debug)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
}

/// Get the log file path for the service (shared across platforms)
fn log_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow!("Could not determine cache directory"))?;
    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;
    Ok(cache_dir.join("daemon.log"))
}

// ============================================================================
// Platform implementations
// ============================================================================

#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod platform {
    use super::*;

    pub fn install() -> Result<()> {
        Err(anyhow!(
            "System service not supported on this platform. Use 'isq daemon run' manually."
        ))
    }

    pub fn uninstall() -> Result<()> {
        Err(anyhow!("System service not supported on this platform"))
    }

    pub fn start() -> Result<()> {
        Err(anyhow!(
            "System service not supported on this platform. Use 'isq daemon run' manually."
        ))
    }

    pub fn stop() -> Result<()> {
        Err(anyhow!("System service not supported on this platform"))
    }

    pub fn status() -> Result<ServiceStatus> {
        Err(anyhow!("System service not supported on this platform"))
    }
}

// ============================================================================
// Public API (delegates to platform module)
// ============================================================================

pub fn install() -> Result<()> {
    platform::install()
}

pub fn uninstall() -> Result<()> {
    platform::uninstall()
}

pub fn start() -> Result<()> {
    platform::start()
}

pub fn stop() -> Result<()> {
    platform::stop()
}

pub fn status() -> Result<ServiceStatus> {
    platform::status()
}
