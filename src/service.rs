use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SERVICE_LABEL: &str = "com.isq.daemon";

/// Get the LaunchAgents directory
fn launch_agents_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents"))
}

/// Get the plist file path
fn plist_path() -> Result<PathBuf> {
    Ok(launch_agents_dir()?.join(format!("{}.plist", SERVICE_LABEL)))
}

/// Get the log file path for the service
fn log_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow!("Could not determine cache directory"))?;
    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;
    Ok(cache_dir.join("daemon.log"))
}

/// Generate the plist content
fn generate_plist() -> Result<String> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_string_lossy();
    let log = log_path()?;
    let log_path_str = log.to_string_lossy();

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>daemon</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
</dict>
</plist>
"#,
        SERVICE_LABEL, exe_path, log_path_str, log_path_str
    ))
}

/// Check if the service is installed
pub fn is_installed() -> Result<bool> {
    let path = plist_path()?;
    Ok(path.exists())
}

/// Check if the service is running
pub fn is_running() -> Result<bool> {
    let output = Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()?;

    Ok(output.status.success())
}

/// Install the daemon as a launchd service
pub fn install() -> Result<()> {
    let plist = plist_path()?;

    // Create LaunchAgents directory if needed
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent)?;
    }

    // Write the plist file
    let content = generate_plist()?;
    fs::write(&plist, content)?;

    // Load the service
    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist)
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to load launchd service"));
    }

    Ok(())
}

/// Uninstall the daemon service
pub fn uninstall() -> Result<()> {
    let plist = plist_path()?;

    if !plist.exists() {
        return Ok(()); // Already uninstalled
    }

    // Unload the service (ignore errors if not loaded)
    let _ = Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(&plist)
        .status();

    // Remove the plist file
    fs::remove_file(&plist)?;

    Ok(())
}

/// Start the service
pub fn start() -> Result<()> {
    // If not installed, install first
    if !is_installed()? {
        install()?;
        return Ok(()); // install() also starts via RunAtLoad
    }

    // If already running, nothing to do
    if is_running()? {
        return Ok(());
    }

    // Start the service
    let status = Command::new("launchctl")
        .args(["start", SERVICE_LABEL])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to start service"));
    }

    Ok(())
}

/// Stop the service
pub fn stop() -> Result<()> {
    if !is_running()? {
        return Ok(()); // Already stopped
    }

    let status = Command::new("launchctl")
        .args(["stop", SERVICE_LABEL])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to stop service"));
    }

    Ok(())
}

/// Restart the service
pub fn restart() -> Result<()> {
    stop()?;
    start()
}

/// Service status information
#[derive(Debug)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pid: Option<u32>,
}

/// Get the service status
pub fn status() -> Result<ServiceStatus> {
    let installed = is_installed()?;

    if !installed {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            pid: None,
        });
    }

    // Parse launchctl list output to get PID
    // When a specific service is queried, launchctl outputs a plist-style format
    let output = Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()?;

    if !output.status.success() {
        return Ok(ServiceStatus {
            installed: true,
            running: false,
            pid: None,
        });
    }

    // Parse the plist-style output to find "PID" = <number>;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid = stdout
        .lines()
        .find(|line| line.contains("\"PID\""))
        .and_then(|line| {
            // Format: "\t\"PID\" = 12345;"
            line.split('=')
                .nth(1)
                .map(|s| s.trim().trim_end_matches(';'))
                .and_then(|s| s.parse::<u32>().ok())
        });

    Ok(ServiceStatus {
        installed: true,
        running: pid.is_some(),
        pid,
    })
}
