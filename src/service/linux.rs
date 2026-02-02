use super::{ServiceStatus, log_path};
use anyhow::{Result, anyhow};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SERVICE_NAME: &str = "isq";

fn systemd_user_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home).join(".config/systemd/user"))
}

fn service_path() -> Result<PathBuf> {
    Ok(systemd_user_dir()?.join(format!("{}.service", SERVICE_NAME)))
}

fn generate_service_file() -> Result<String> {
    let exe = std::env::current_exe()?;
    let exe_path = exe.to_string_lossy();
    let log = log_path()?;
    let log_path_str = log.to_string_lossy();

    Ok(format!(
        r#"[Unit]
Description=isq daemon - issue queue sync service
After=network.target

[Service]
Type=simple
ExecStart={} daemon run
Restart=always
RestartSec=5
StandardOutput=append:{}
StandardError=append:{}

[Install]
WantedBy=default.target
"#,
        exe_path, log_path_str, log_path_str
    ))
}

fn is_installed() -> Result<bool> {
    let path = service_path()?;
    Ok(path.exists())
}

fn is_running() -> Result<bool> {
    let output = Command::new("systemctl")
        .args(["--user", "is-active", SERVICE_NAME])
        .output()?;
    Ok(output.status.success())
}

pub fn install() -> Result<()> {
    let service = service_path()?;

    if let Some(parent) = service.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = generate_service_file()?;
    fs::write(&service, content)?;

    // Reload systemd to pick up new service
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    // Enable and start the service
    let status = Command::new("systemctl")
        .args(["--user", "enable", "--now", SERVICE_NAME])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to enable systemd service"));
    }

    Ok(())
}

pub fn uninstall() -> Result<()> {
    let service = service_path()?;

    if !service.exists() {
        return Ok(());
    }

    // Disable and stop the service
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", SERVICE_NAME])
        .status();

    // Remove the service file
    fs::remove_file(&service)?;

    // Reload systemd
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

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

    let status = Command::new("systemctl")
        .args(["--user", "start", SERVICE_NAME])
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

    let status = Command::new("systemctl")
        .args(["--user", "stop", SERVICE_NAME])
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
/// because the service file still points to the old binary path.
pub fn reinstall() -> Result<()> {
    let service = service_path()?;

    // Stop the service if running
    let _ = Command::new("systemctl")
        .args(["--user", "stop", SERVICE_NAME])
        .status();

    // Write new service file with current binary path
    if let Some(parent) = service.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = generate_service_file()?;
    fs::write(&service, content)?;

    // Reload systemd to pick up changes
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    // Start the service
    let status = Command::new("systemctl")
        .args(["--user", "start", SERVICE_NAME])
        .status()?;

    if !status.success() {
        return Err(anyhow!("Failed to start service"));
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

    // Get service properties
    let output = Command::new("systemctl")
        .args([
            "--user",
            "show",
            SERVICE_NAME,
            "--property=ActiveState,MainPID",
        ])
        .output()?;

    if !output.status.success() {
        return Ok(ServiceStatus {
            installed: true,
            running: false,
            pid: None,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut running = false;
    let mut pid = None;

    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("ActiveState=") {
            running = value == "active";
        }
        if let Some(value) = line.strip_prefix("MainPID=")
            && let Ok(p) = value.parse::<u32>()
            && p > 0
        {
            pid = Some(p);
        }
    }

    Ok(ServiceStatus {
        installed: true,
        running,
        pid,
    })
}
