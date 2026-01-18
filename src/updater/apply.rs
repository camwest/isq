//! Update application logic

use anyhow::Result;

use crate::install::{self, InstallMethod};

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
