//! Binary version detection for daemon auto-restart

use anyhow::{Result, anyhow};

/// Parse version from `isq --version` output.
/// Format: "isq 0.1.0" or "isq 0.1.0 (standalone, auto-updates enabled)"
pub(crate) fn parse_version_from_output(output: &str) -> Result<String> {
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
