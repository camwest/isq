use anyhow::{anyhow, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
// macOS (launchd)
// ============================================================================

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

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

    pub fn is_installed() -> Result<bool> {
        let path = plist_path()?;
        Ok(path.exists())
    }

    pub fn is_running() -> Result<bool> {
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
}

// ============================================================================
// Linux (systemd)
// ============================================================================

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

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

    pub fn is_installed() -> Result<bool> {
        let path = service_path()?;
        Ok(path.exists())
    }

    pub fn is_running() -> Result<bool> {
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
            .args(["--user", "show", SERVICE_NAME, "--property=ActiveState,MainPID"])
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
            if let Some(value) = line.strip_prefix("MainPID=") {
                if let Ok(p) = value.parse::<u32>() {
                    if p > 0 {
                        pid = Some(p);
                    }
                }
            }
        }

        Ok(ServiceStatus {
            installed: true,
            running,
            pid,
        })
    }
}

// ============================================================================
// Unsupported platforms
// ============================================================================

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub fn is_installed() -> Result<bool> {
        Err(anyhow!("System service not supported on this platform"))
    }

    pub fn is_running() -> Result<bool> {
        Err(anyhow!("System service not supported on this platform"))
    }

    pub fn install() -> Result<()> {
        Err(anyhow!("System service not supported on this platform. Use 'isq daemon run' manually."))
    }

    pub fn uninstall() -> Result<()> {
        Err(anyhow!("System service not supported on this platform"))
    }

    pub fn start() -> Result<()> {
        Err(anyhow!("System service not supported on this platform. Use 'isq daemon run' manually."))
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

pub fn is_installed() -> Result<bool> {
    platform::is_installed()
}

pub fn is_running() -> Result<bool> {
    platform::is_running()
}

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

pub fn restart() -> Result<()> {
    stop()?;
    start()
}

pub fn status() -> Result<ServiceStatus> {
    platform::status()
}
