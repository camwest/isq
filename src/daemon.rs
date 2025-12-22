use anyhow::Result;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::db;
use crate::forge::{get_forge_for_repo, CreateIssueRequest, Forge};
use crate::repo::Repo;

// Sync all repos at this interval
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

/// Spawn the daemon process (watches all repos in watched_repos table)
pub fn spawn() -> Result<()> {
    // Check if already running
    if let Some(pid) = is_running()? {
        eprintln!("Daemon already running (PID {})", pid);
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let log_file = log_path()?;

    // Spawn detached process
    let child = Command::new(&exe)
        .args(["daemon", "run"])
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

/// Run the daemon sync loop (watches all repos)
///
/// Syncs all watched repos every SYNC_INTERVAL_SECS.
/// Repos are sorted by last_accessed (most recent first) so that if we can't
/// finish all repos before the next cycle (due to rate limits or too many repos),
/// the ones you're actively using get priority.
pub async fn run_loop() -> Result<()> {
    eprintln!("[daemon] Starting sync loop (interval: {}s)", SYNC_INTERVAL_SECS);

    // Clean up stale repo entries on startup
    if let Ok(conn) = db::open() {
        if let Ok(removed) = db::cleanup_stale_repos(&conn) {
            if removed > 0 {
                eprintln!("[daemon] Cleaned up {} stale repo entries", removed);
            }
        }
    }

    loop {
        let conn = db::open()?;
        let watched = db::list_watched_repos(&conn)?;
        // list_watched_repos already returns sorted by last_accessed DESC

        if watched.is_empty() {
            eprintln!("[daemon] No repos to watch, waiting...");
        } else {
            eprintln!("[daemon] Syncing {} repos...", watched.len());
            for repo in &watched {
                if let Err(e) = sync_once(&repo.repo).await {
                    eprintln!("[daemon] Sync error for {}: {}", repo.repo, e);
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(SYNC_INTERVAL_SECS)).await;
    }
}

/// Sync a single repo by its local path.
///
/// Looks up the repo_link to determine which forge to use,
/// then syncs issues from that forge.
async fn sync_once(repo_path: &str) -> Result<()> {
    // Look up the repo link to get forge info
    let (forge, link) = get_forge_for_repo(repo_path)?;

    // Parse the forge_repo (e.g., "owner/repo" for GitHub)
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }

    let repo = Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let conn = db::open()?;

    // First, process any pending operations
    // Note: pending_ops are keyed by forge_repo for consistency
    let pending_ops = db::load_pending_ops(&conn, &link.forge_repo)?;
    if !pending_ops.is_empty() {
        eprintln!("[daemon] Processing {} pending operations...", pending_ops.len());
        let synced = process_pending_ops(forge.as_ref(), &repo, &conn, &pending_ops).await;
        if synced > 0 {
            eprintln!("[daemon] Synced {} pending operations", synced);
        }
    }

    // Then sync issues from remote
    let issues = forge.list_issues(&repo).await?;
    db::save_issues(&conn, &link.forge_repo, &issues)?;

    // Sync comments
    let comments = forge.list_all_comments(&repo).await?;
    db::save_comments(&conn, &link.forge_repo, &comments)?;

    eprintln!(
        "[daemon] Synced {} issues and {} comments for {}",
        issues.len(),
        comments.len(),
        link.forge_repo
    );

    Ok(())
}

/// Process pending operations and return count of successful syncs
async fn process_pending_ops(
    forge: &dyn Forge,
    repo: &Repo,
    conn: &rusqlite::Connection,
    ops: &[db::PendingOp],
) -> usize {
    let mut synced = 0;

    for op in ops {
        let result = execute_pending_op(forge, repo, op).await;

        match result {
            Ok(()) => {
                // Operation succeeded, remove from queue
                if let Err(e) = db::complete_op(conn, op.id) {
                    eprintln!("[daemon] Failed to mark op {} complete: {}", op.id, e);
                }
                synced += 1;
            }
            Err(e) => {
                // Check if this is a conflict (server state changed)
                let err_str = e.to_string();
                if err_str.contains("404") || err_str.contains("422") || err_str.contains("409") {
                    // Conflict or resource not found - server wins, discard operation
                    eprintln!(
                        "[daemon] Conflict for {} op on {}: {} (discarding)",
                        op.op_type, repo.full_name(), e
                    );
                    if let Err(e) = db::complete_op(conn, op.id) {
                        eprintln!("[daemon] Failed to discard op {}: {}", op.id, e);
                    }
                    synced += 1; // Count as processed
                } else {
                    // Network or other transient error - leave in queue for retry
                    eprintln!(
                        "[daemon] Failed {} op, will retry: {}",
                        op.op_type, e
                    );
                }
            }
        }
    }

    synced
}

/// Execute a single pending operation
async fn execute_pending_op(
    forge: &dyn Forge,
    repo: &Repo,
    op: &db::PendingOp,
) -> Result<()> {
    let payload: serde_json::Value = serde_json::from_str(&op.payload)?;

    match op.op_type.as_str() {
        "create" => {
            let req = CreateIssueRequest {
                title: payload["title"].as_str().unwrap_or("").to_string(),
                body: payload["body"].as_str().map(|s| s.to_string()),
                labels: payload["labels"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            let issue = forge.create_issue(repo, req).await?;
            eprintln!("[daemon] Created #{} {}", issue.number, issue.title);
        }
        "comment" => {
            let issue_number = payload["issue_number"].as_u64().unwrap_or(0);
            let body = payload["body"].as_str().unwrap_or("");
            forge.create_comment(repo, issue_number, body).await?;
            eprintln!("[daemon] Added comment to #{}", issue_number);
        }
        "close" => {
            let issue_number = payload["issue_number"].as_u64().unwrap_or(0);
            forge.close_issue(repo, issue_number).await?;
            eprintln!("[daemon] Closed #{}", issue_number);
        }
        "reopen" => {
            let issue_number = payload["issue_number"].as_u64().unwrap_or(0);
            forge.reopen_issue(repo, issue_number).await?;
            eprintln!("[daemon] Reopened #{}", issue_number);
        }
        "label_add" => {
            let issue_number = payload["issue_number"].as_u64().unwrap_or(0);
            let label = payload["label"].as_str().unwrap_or("");
            forge.add_label(repo, issue_number, label).await?;
            eprintln!("[daemon] Added label '{}' to #{}", label, issue_number);
        }
        "label_remove" => {
            let issue_number = payload["issue_number"].as_u64().unwrap_or(0);
            let label = payload["label"].as_str().unwrap_or("");
            forge.remove_label(repo, issue_number, label).await?;
            eprintln!("[daemon] Removed label '{}' from #{}", label, issue_number);
        }
        "assign" => {
            let issue_number = payload["issue_number"].as_u64().unwrap_or(0);
            let assignee = payload["assignee"].as_str().unwrap_or("");
            forge.assign_issue(repo, issue_number, assignee).await?;
            eprintln!("[daemon] Assigned @{} to #{}", assignee, issue_number);
        }
        _ => {
            anyhow::bail!("Unknown op type: {}", op.op_type);
        }
    }

    Ok(())
}
