use super::{ServiceStatus, log_path};
use anyhow::{Result, anyhow};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SERVICE_LABEL: &str = "com.isq.daemon";

fn launch_agents_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home).join("Library/LaunchAgents"))
}

fn plist_path() -> Result<PathBuf> {
    Ok(launch_agents_dir()?.join(format!("{}.plist", SERVICE_LABEL)))
}

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

fn is_installed() -> Result<bool> {
    let path = plist_path()?;
    Ok(path.exists())
}

fn is_running() -> Result<bool> {
    let output = Command::new("launchctl")
        .args(["list", SERVICE_LABEL])
        .output()?;
    Ok(output.status.success())
}

pub fn install() -> Result<()> {
    let plist = plist_path()?;

    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = generate_plist()?;
    fs::write(&plist, content)?;

    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist)
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to load launchd service"));
    }

    Ok(())
}

pub fn uninstall() -> Result<()> {
    let plist = plist_path()?;

    if !plist.exists() {
        return Ok(());
    }

    let _ = Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(&plist)
        .status();

    fs::remove_file(&plist)?;
    Ok(())
}

pub fn start() -> Result<()> {
    if !is_installed()? {
        install()?;
        return Ok(());
    }

    if is_running()? {
        return Ok(());
    }

    let status = Command::new("launchctl")
        .args(["start", SERVICE_LABEL])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to start service"));
    }

    Ok(())
}

pub fn stop() -> Result<()> {
    if !is_running()? {
        return Ok(());
    }

    let status = Command::new("launchctl")
        .args(["stop", SERVICE_LABEL])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to stop service"));
    }

    Ok(())
}

/// Reinstall the service with the current binary path.
///
/// This is needed when the binary has been updated and we need to restart
/// the daemon with the new version. Simply stopping and starting won't work
/// because the plist still points to the old binary path.
pub fn reinstall() -> Result<()> {
    // Unload if running (ignore errors - may not be running)
    let plist = plist_path()?;
    if plist.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist)
            .status();
    }

    // Write new plist with current binary path
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = generate_plist()?;
    fs::write(&plist, content)?;

    // Load the updated plist
    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist)
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to load launchd service"));
    }

    Ok(())
}

pub fn status() -> Result<ServiceStatus> {
    let installed = is_installed()?;

    if !installed {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            pid: None,
        });
    }

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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid = stdout
        .lines()
        .find(|line| line.contains("\"PID\""))
        .and_then(|line| {
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
