use anyhow::Result;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::db;
use crate::forges::{get_forge_for_repo, CreateIssueRequest, Forge};
use crate::repo::Repo;

// Sync all repos at this interval
const SYNC_INTERVAL_SECS: u64 = 30;
const MAX_BACKOFF_SECS: u64 = 3600; // Max 1 hour backoff

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
#[cfg(unix)]
fn acquire_lock() -> Result<File> {
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

#[cfg(not(unix))]
fn acquire_lock() -> Result<File> {
    // On Windows, just create the lock file (basic protection)
    let path = lock_path()?;
    Ok(File::create(&path)?)
}

/// Check if the daemon lock is held by another process
#[cfg(unix)]
pub fn is_locked() -> Result<bool> {
    use std::os::unix::io::AsRawFd;

    let path = lock_path()?;
    if !path.exists() {
        return Ok(false);
    }

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return Ok(false),
    };

    // Try to acquire lock (non-blocking)
    let fd = file.as_raw_fd();
    let result = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };

    if result != 0 {
        // Lock is held by another process
        Ok(true)
    } else {
        // We got the lock, release it immediately
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        Ok(false)
    }
}

#[cfg(not(unix))]
pub fn is_locked() -> Result<bool> {
    // Fallback to PID-based check on non-Unix
    Ok(is_running()?.is_some())
}

/// Per-repo sync state for backoff tracking
struct RepoSyncState {
    consecutive_failures: u32,
    next_attempt: Instant,
}

/// Calculate backoff duration with exponential increase and jitter
fn calculate_backoff(failures: u32) -> Duration {
    let base_secs = SYNC_INTERVAL_SECS;

    // Exponential: 30s, 60s, 120s, 240s, ... up to MAX_BACKOFF_SECS
    let backoff_secs = base_secs * 2u64.pow(failures.min(6));
    let capped_secs = backoff_secs.min(MAX_BACKOFF_SECS);

    // Add jitter: ±25%
    let jitter = (rand::random::<f64>() - 0.5) * 0.5; // -0.25 to +0.25
    let jittered = capped_secs as f64 * (1.0 + jitter);

    Duration::from_secs_f64(jittered.max(1.0))
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
    // Check if lock is held (more reliable than PID check)
    if is_locked()? {
        if let Some(pid) = is_running()? {
            anyhow::bail!("Daemon already running (PID {})", pid);
        } else {
            anyhow::bail!("Daemon already running (lock held)");
        }
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
    // Acquire exclusive lock FIRST - prevents multiple instances
    let _lock = acquire_lock()?;
    eprintln!("[daemon] Acquired exclusive lock");

    // Write PID file after acquiring lock
    let pid_file = pid_path()?;
    let mut f = File::create(&pid_file)?;
    writeln!(f, "{}", std::process::id())?;
    drop(f);

    eprintln!("[daemon] Starting sync loop (interval: {}s)", SYNC_INTERVAL_SECS);

    // Clean up stale repo entries on startup
    if let Ok(conn) = db::open() {
        if let Ok(removed) = db::cleanup_stale_repos(&conn) {
            if removed > 0 {
                eprintln!("[daemon] Cleaned up {} stale repo entries", removed);
            }
        }
    }

    // Track per-repo backoff state
    let mut repo_states: HashMap<String, RepoSyncState> = HashMap::new();

    loop {
        let conn = db::open()?;
        let watched = db::list_watched_repos(&conn)?;
        // list_watched_repos already returns sorted by last_accessed DESC

        if watched.is_empty() {
            eprintln!("[daemon] No repos to watch, waiting...");
        } else {
            let now = Instant::now();
            let mut synced = 0;
            let mut skipped = 0;

            for repo in &watched {
                // Check if this repo is in backoff
                if let Some(state) = repo_states.get(&repo.repo) {
                    if now < state.next_attempt {
                        skipped += 1;
                        continue;
                    }
                }

                match sync_once(&repo.repo).await {
                    Ok(()) => {
                        // Success - reset backoff state
                        repo_states.remove(&repo.repo);
                        synced += 1;
                    }
                    Err(e) => {
                        eprintln!("[daemon] Sync error for {}: {}", repo.repo, e);

                        // Update backoff state
                        let state = repo_states.entry(repo.repo.clone()).or_insert(RepoSyncState {
                            consecutive_failures: 0,
                            next_attempt: now,
                        });
                        state.consecutive_failures += 1;
                        let backoff = calculate_backoff(state.consecutive_failures);
                        state.next_attempt = now + backoff;

                        eprintln!(
                            "[daemon] {} in backoff for {:.0}s (failures: {})",
                            repo.repo,
                            backoff.as_secs_f64(),
                            state.consecutive_failures
                        );
                    }
                }
            }

            if synced > 0 || skipped > 0 {
                eprintln!(
                    "[daemon] Cycle complete: {} synced, {} in backoff",
                    synced, skipped
                );
            }
        }

        // Add jitter to sleep interval to prevent synchronized requests
        let jitter = (rand::random::<f64>() - 0.5) * 0.2; // ±10%
        let sleep_secs = SYNC_INTERVAL_SECS as f64 * (1.0 + jitter);
        tokio::time::sleep(Duration::from_secs_f64(sleep_secs)).await;
    }
}

/// Sync a single repo by its local path.
///
/// Looks up the repo_link to determine which forge to use,
/// then syncs issues from that forge.
async fn sync_once(repo_path: &str) -> Result<()> {
    // Look up the repo link to get forge info
    let (forge, link) = get_forge_for_repo(repo_path)?;

    let conn = db::open()?;

    // Check if we're rate limited for this forge
    if db::is_rate_limited(&conn, &link.forge_type)? {
        if let Some(state) = db::get_rate_limit_state(&conn, &link.forge_type)? {
            if let Some(reset_at) = state.reset_at {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let wait_secs = reset_at - now;
                eprintln!(
                    "[daemon] {} rate limited, skipping {} (resets in {}s)",
                    link.forge_type, link.forge_repo, wait_secs
                );
                return Ok(());
            }
        }
    }

    // Parse the forge_repo (e.g., "owner/repo" for GitHub)
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }

    let repo = Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

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
    let issues = match forge.list_issues(&repo).await {
        Ok(issues) => issues,
        Err(e) => {
            // Check if this is a rate limit error
            let err_str = e.to_string();
            if err_str.contains("rate limit") || err_str.contains("403") {
                // Try to get rate limit info from the forge
                if let Ok(Some(rate_info)) = forge.get_rate_limit().await {
                    db::set_rate_limit_state(
                        &conn,
                        &link.forge_type,
                        Some(rate_info.reset_at),
                        Some(&err_str),
                    )?;
                    eprintln!(
                        "[daemon] {} rate limited until {} (remaining: {})",
                        link.forge_type,
                        rate_info.reset_at,
                        rate_info.remaining
                    );
                } else {
                    // Fallback: use 60 second backoff if we can't get rate limit info
                    let reset_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64
                        + 60;
                    db::set_rate_limit_state(&conn, &link.forge_type, Some(reset_at), Some(&err_str))?;
                }
            }
            return Err(e);
        }
    };
    db::save_issues(&conn, &link.forge_repo, &issues)?;

    // Sync comments
    let comments = match forge.list_all_comments(&repo).await {
        Ok(comments) => comments,
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("rate limit") || err_str.contains("403") {
                if let Ok(Some(rate_info)) = forge.get_rate_limit().await {
                    db::set_rate_limit_state(
                        &conn,
                        &link.forge_type,
                        Some(rate_info.reset_at),
                        Some(&err_str),
                    )?;
                } else {
                    let reset_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64
                        + 60;
                    db::set_rate_limit_state(&conn, &link.forge_type, Some(reset_at), Some(&err_str))?;
                }
            }
            return Err(e);
        }
    };
    db::save_comments(&conn, &link.forge_repo, &comments)?;

    // Sync was successful - fetch and save rate limit info
    if let Ok(Some(rate_info)) = forge.get_rate_limit().await {
        db::update_rate_limit_budget(
            &conn,
            &link.forge_type,
            rate_info.limit,
            rate_info.remaining,
            rate_info.reset_at,
        )?;
    }

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
                goal_id: payload["goal_id"].as_str().map(|s| s.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_backoff_base_case() {
        // 0 failures = base interval (30s) with jitter
        let backoff = calculate_backoff(0);
        let secs = backoff.as_secs_f64();

        // Base is 30s, jitter is ±25%, so range is 22.5 to 37.5
        assert!(secs >= 22.5, "backoff {} too low for 0 failures", secs);
        assert!(secs <= 37.5, "backoff {} too high for 0 failures", secs);
    }

    #[test]
    fn test_calculate_backoff_exponential_growth() {
        // Test that backoff grows exponentially (within jitter bounds)
        // 1 failure = 60s base, 2 = 120s, 3 = 240s, etc.

        let b1 = calculate_backoff(1);
        let b2 = calculate_backoff(2);
        let b3 = calculate_backoff(3);

        // With ±25% jitter: 1 failure = 45-75s, 2 = 90-150s, 3 = 180-300s
        assert!(b1.as_secs_f64() >= 45.0 && b1.as_secs_f64() <= 75.0,
            "1 failure backoff {} out of range", b1.as_secs_f64());
        assert!(b2.as_secs_f64() >= 90.0 && b2.as_secs_f64() <= 150.0,
            "2 failure backoff {} out of range", b2.as_secs_f64());
        assert!(b3.as_secs_f64() >= 180.0 && b3.as_secs_f64() <= 300.0,
            "3 failure backoff {} out of range", b3.as_secs_f64());
    }

    #[test]
    fn test_calculate_backoff_caps_at_max() {
        // Exponent caps at 6: 30 * 2^6 = 1920s max
        // With ±25% jitter: 1440 to 2400
        let backoff = calculate_backoff(10);
        let secs = backoff.as_secs_f64();

        assert!(secs >= 1440.0, "max backoff {} too low", secs);
        assert!(secs <= 2400.0, "max backoff {} too high", secs);
    }

    #[test]
    fn test_calculate_backoff_very_high_failures() {
        // Even with extreme failures, should not overflow and should cap at 1920s
        let backoff = calculate_backoff(100);
        let secs = backoff.as_secs_f64();

        // Should be capped at 1920s with ±25% jitter = 1440 to 2400
        assert!(secs >= 1440.0 && secs <= 2400.0,
            "extreme failure backoff {} should be capped", secs);
    }

    #[test]
    fn test_calculate_backoff_has_jitter() {
        // Run multiple times and verify we get different values (jitter working)
        let mut values: Vec<f64> = Vec::new();
        for _ in 0..10 {
            values.push(calculate_backoff(2).as_secs_f64());
        }

        // Check that not all values are identical (jitter is applied)
        let first = values[0];
        let has_variation = values.iter().any(|&v| (v - first).abs() > 0.001);
        assert!(has_variation, "backoff should have jitter variation");
    }
}
