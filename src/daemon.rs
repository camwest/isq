use anyhow::Result;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::auth;
use crate::db;
use crate::github::GitHubClient;
use crate::repo::Repo;

const SYNC_INTERVAL_SECS: u64 = 30;

/// Get the daemon PID file path
pub fn pid_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;

    Ok(cache_dir.join("daemon.pid"))
}

/// Get the daemon log file path
pub fn log_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;

    Ok(cache_dir.join("daemon.log"))
}

/// Check if daemon is running
pub fn is_running() -> Result<Option<u32>> {
    let pid_file = pid_path()?;

    if !pid_file.exists() {
        return Ok(None);
    }

    let pid_str = fs::read_to_string(&pid_file)?;
    let pid: u32 = pid_str.trim().parse()?;

    // Check if process is still alive
    #[cfg(unix)]
    {
        use std::process::Command;
        let status = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match status {
            Ok(s) if s.success() => Ok(Some(pid)),
            _ => {
                // Process is dead, clean up PID file
                let _ = fs::remove_file(&pid_file);
                Ok(None)
            }
        }
    }

    #[cfg(not(unix))]
    {
        // On Windows, just assume it's running if PID file exists
        Ok(Some(pid))
    }
}

/// Spawn the daemon process
pub fn spawn(repo: &Repo) -> Result<()> {
    // Check if already running
    if let Some(pid) = is_running()? {
        eprintln!("Daemon already running (PID {})", pid);
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let log_file = log_path()?;

    // Spawn detached process
    let child = Command::new(&exe)
        .args(["daemon", "run", "--repo", &repo.full_name()])
        .stdout(Stdio::from(fs::File::create(&log_file)?))
        .stderr(Stdio::from(fs::File::options().append(true).open(&log_file)?))
        .stdin(Stdio::null())
        .spawn()?;

    // Write PID file
    let pid_file = pid_path()?;
    let mut f = fs::File::create(&pid_file)?;
    writeln!(f, "{}", child.id())?;

    eprintln!("✓ Daemon started (PID {})", child.id());

    Ok(())
}

/// Stop the daemon
pub fn stop() -> Result<()> {
    let pid_file = pid_path()?;

    if let Some(pid) = is_running()? {
        #[cfg(unix)]
        {
            Command::new("kill")
                .arg(pid.to_string())
                .status()?;
        }

        let _ = fs::remove_file(&pid_file);
        eprintln!("✓ Daemon stopped (PID {})", pid);
    } else {
        eprintln!("Daemon not running");
    }

    Ok(())
}

/// Run the daemon sync loop (called when spawned)
pub async fn run_loop(repo_name: &str) -> Result<()> {
    eprintln!("[daemon] Starting sync loop for {}", repo_name);

    loop {
        if let Err(e) = sync_once(repo_name).await {
            eprintln!("[daemon] Sync error: {}", e);
        }

        tokio::time::sleep(Duration::from_secs(SYNC_INTERVAL_SECS)).await;
    }
}

async fn sync_once(repo_name: &str) -> Result<()> {
    let token = auth::get_gh_token()?;

    let parts: Vec<&str> = repo_name.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repo name: {}", repo_name);
    }

    let repo = Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let client = GitHubClient::new(token);
    let issues = client.list_issues(&repo).await?;

    let conn = db::open()?;
    db::save_issues(&conn, repo_name, &issues)?;

    eprintln!(
        "[daemon] Synced {} issues for {}",
        issues.len(),
        repo_name
    );

    Ok(())
}
