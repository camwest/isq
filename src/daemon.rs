use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::db;
use crate::forges::{get_forge_for_repo, CreateIssueRequest, FetchResult, Forge};
use crate::repo::Repo;

// Sync all repos at this interval
const SYNC_INTERVAL_SECS: u64 = 15; // Reduced from 30s since incremental is cheaper
const MAX_BACKOFF_SECS: u64 = 3600; // Max 1 hour backoff
const FULL_SYNC_INTERVAL_HOURS: i64 = 1; // Hours between full reconciliation syncs
const MAX_CONCURRENT_SYNCS: usize = 4; // Max repos to sync in parallel
const FULL_SYNC_RETRY_COOLDOWN_MINS: i64 = 15; // Minutes to wait after failed full sync attempt
const VERSION_CHECK_INTERVAL_SECS: u64 = 300; // 5 minutes - check if binary was updated

/// Information about the running daemon, stored in the PID file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub version: String,
    pub started_at: DateTime<Utc>,
}

/// Get the daemon PID file path
pub fn pid_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    let cache_dir = dirs.cache_dir();
    fs::create_dir_all(cache_dir)?;

    Ok(cache_dir.join("daemon.pid"))
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

/// Write daemon info to the PID file in JSON format.
fn write_daemon_info(info: &DaemonInfo) -> Result<()> {
    let pid_file = pid_path()?;
    let content = serde_json::to_string_pretty(info)?;
    fs::write(&pid_file, content)?;
    Ok(())
}

/// Read daemon info from the PID file.
///
/// Returns None if file doesn't exist or is invalid JSON.
pub fn read_daemon_info() -> Result<Option<DaemonInfo>> {
    let pid_file = pid_path()?;

    if !pid_file.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&pid_file)?;
    Ok(serde_json::from_str(&content).ok())
}

/// Per-repo sync state for backoff tracking
struct RepoSyncState {
    consecutive_failures: u32,
    next_attempt: Instant,
}

/// Result of a single repo sync attempt
enum SyncResult {
    Success,
    Skipped,
    Error(anyhow::Error),
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

/// Run the daemon sync loop (watches all repos)
///
/// Syncs all watched repos every SYNC_INTERVAL_SECS.
/// Repos are sorted by last_accessed (most recent first) so that if we can't
/// finish all repos before the next cycle (due to rate limits or too many repos),
/// the ones you're actively using get priority.
///
/// Also checks every VERSION_CHECK_INTERVAL_SECS if the binary on disk has been
/// updated, and exits gracefully to allow the service manager to restart with
/// the new version.
pub async fn run_loop() -> Result<()> {
    // Acquire exclusive lock FIRST - prevents multiple instances
    let _lock = acquire_lock()?;
    eprintln!("[daemon] Acquired exclusive lock");

    // Write daemon info (JSON format with version)
    let daemon_info = DaemonInfo {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        started_at: Utc::now(),
    };
    write_daemon_info(&daemon_info)?;

    eprintln!(
        "[daemon] Starting sync loop (sync: {}s, version check: {}s)",
        SYNC_INTERVAL_SECS, VERSION_CHECK_INTERVAL_SECS
    );

    // Clean up stale repo entries on startup
    if let Ok(conn) = db::open() {
        if let Ok(removed) = db::cleanup_stale_repos(&conn) {
            if removed > 0 {
                eprintln!("[daemon] Cleaned up {} stale repo entries", removed);
            }
        }
    }

    // Track per-repo backoff state (thread-safe for parallel sync)
    let repo_states: Arc<Mutex<HashMap<String, RepoSyncState>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Create intervals for sync and version check
    let mut sync_interval = tokio::time::interval(Duration::from_secs(SYNC_INTERVAL_SECS));
    let mut version_interval = tokio::time::interval(Duration::from_secs(VERSION_CHECK_INTERVAL_SECS));

    // Skip the first immediate tick for version check (don't check on startup)
    version_interval.tick().await;

    loop {
        tokio::select! {
            _ = sync_interval.tick() => {
                perform_sync_cycle(&repo_states).await;
            }
            _ = version_interval.tick() => {
                match crate::updater::is_binary_updated().await {
                    Ok(true) => {
                        eprintln!("[daemon] Binary updated, exiting for restart...");
                        return Ok(()); // Service manager will restart us
                    }
                    Ok(false) => {} // Same version, continue
                    Err(e) => {
                        eprintln!("[daemon] Version check failed: {} (continuing)", e);
                    }
                }
            }
        }
    }
}

/// Perform a single sync cycle for all watched repos.
async fn perform_sync_cycle(repo_states: &Arc<Mutex<HashMap<String, RepoSyncState>>>) {
    let conn = match db::open() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[daemon] Failed to open database: {}", e);
            return;
        }
    };

    let watched = match db::list_watched_repos(&conn) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[daemon] Failed to list watched repos: {}", e);
            return;
        }
    };

    if watched.is_empty() {
        eprintln!("[daemon] No repos to watch, waiting...");
        return;
    }

    let now = Instant::now();

    // Sync repos in parallel with bounded concurrency
    let results: Vec<(String, SyncResult)> = stream::iter(watched.iter())
        .map(|repo| {
            let states = Arc::clone(repo_states);
            let repo_path = repo.repo.clone();
            async move {
                // Check if this repo is in backoff
                {
                    let states = states.lock().await;
                    if let Some(state) = states.get(&repo_path) {
                        if Instant::now() < state.next_attempt {
                            return (repo_path, SyncResult::Skipped);
                        }
                    }
                }

                // Sync the repo
                match sync_once(&repo_path).await {
                    Ok(()) => (repo_path, SyncResult::Success),
                    Err(e) => (repo_path, SyncResult::Error(e)),
                }
            }
        })
        .buffer_unordered(MAX_CONCURRENT_SYNCS)
        .collect()
        .await;

    // Update backoff states based on results
    let mut synced = 0;
    let mut skipped = 0;
    {
        let mut states = repo_states.lock().await;
        for (repo_path, result) in results {
            match result {
                SyncResult::Success => {
                    states.remove(&repo_path);
                    synced += 1;
                }
                SyncResult::Skipped => {
                    skipped += 1;
                }
                SyncResult::Error(e) => {
                    eprintln!("[daemon] Sync error for {}: {}", repo_path, e);

                    let state =
                        states.entry(repo_path.clone()).or_insert(RepoSyncState {
                            consecutive_failures: 0,
                            next_attempt: now,
                        });
                    state.consecutive_failures += 1;
                    let backoff = calculate_backoff(state.consecutive_failures);
                    state.next_attempt = now + backoff;

                    eprintln!(
                        "[daemon] {} in backoff for {:.0}s (failures: {})",
                        repo_path,
                        backoff.as_secs_f64(),
                        state.consecutive_failures
                    );
                }
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

/// Determine if we need a full sync based on sync state
/// Returns (should_do_full_sync, in_cooldown)
fn should_do_full_sync(sync_state: &Option<db::SyncState>, has_cursor: bool) -> (bool, bool) {
    let Some(state) = sync_state else {
        return (true, false); // First sync ever
    };

    let now = Utc::now();

    // Check if we're in cooldown from a recent attempt (prevents retry storms)
    let in_cooldown = state.last_full_sync_attempt_at.as_ref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| now - t.with_timezone(&Utc) <= ChronoDuration::minutes(FULL_SYNC_RETRY_COOLDOWN_MINS))
        .unwrap_or(false);

    if in_cooldown {
        return (false, true);
    }

    // Not in cooldown - check if we need full sync
    if !has_cursor {
        return (true, false); // No cursor available, must do full sync
    }

    // Check if successful full sync is stale (> 1 hour)
    let needs_full = state.last_full_sync_at.as_ref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| now - t.with_timezone(&Utc) > ChronoDuration::hours(FULL_SYNC_INTERVAL_HOURS))
        .unwrap_or(true); // Never successfully completed

    (needs_full, false)
}

/// Handle rate limit errors by recording state
async fn handle_rate_limit_error(
    conn: &rusqlite::Connection,
    forge_type: &str,
    forge: &dyn Forge,
    e: &anyhow::Error,
) -> Result<()> {
    let err_str = e.to_string();
    if err_str.contains("rate limit") || err_str.contains("429") || err_str.contains("403") {
        if let Ok(Some(rate_info)) = forge.get_rate_limit().await {
            db::set_rate_limit_state(conn, forge_type, Some(rate_info.reset_at), Some(&err_str))?;
            eprintln!(
                "[daemon] {} rate limited until {} (remaining: {})",
                forge_type, rate_info.reset_at, rate_info.remaining
            );
        } else {
            let reset_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64
                + 60;
            db::set_rate_limit_state(conn, forge_type, Some(reset_at), Some(&err_str))?;
        }
    }
    Ok(())
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

    // === ISSUES ===
    // Calculate cursor for incremental sync (subtract 1 second for safety buffer)
    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
    let issues_cursor = sync_state.as_ref()
        .and_then(|s| s.issues_last_sync.as_ref())
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc) - ChronoDuration::seconds(1));

    // Determine if we need full sync based on sync state and cursor availability
    let (needs_full_sync, in_cooldown) = should_do_full_sync(&sync_state, issues_cursor.is_some());

    // If in cooldown with no cursor, skip this sync entirely
    if in_cooldown && issues_cursor.is_none() {
        eprintln!(
            "[daemon] Skipping {} - full sync in cooldown, no cursor for incremental",
            link.forge_repo
        );
        return Ok(());
    }

    let issues_result = if needs_full_sync {
        match forge.list_issues(&repo).await {
            Ok(result) => result,
            Err(e) => {
                handle_rate_limit_error(&conn, &link.forge_type, forge.as_ref(), &e).await?;
                return Err(e);
            }
        }
    } else {
        match forge.list_issues_since(&repo, issues_cursor.unwrap()).await {
            Ok(result) => result,
            Err(e) => {
                handle_rate_limit_error(&conn, &link.forge_type, forge.as_ref(), &e).await?;
                return Err(e);
            }
        }
    };

    let mut issues = issues_result.items;

    // Apply priority from repo config (each forge handles its own logic)
    if let Ok(Some(config)) = crate::config::load_repo_config(std::path::Path::new(repo_path)) {
        forge.apply_priority_config(&mut issues, &config.priority);
    }

    let issues_stats = db::save_issues(
        &conn,
        &link.forge_repo,
        &issues,
        needs_full_sync,
        issues_result.is_complete,
    )?;

    // === COMMENTS ===
    let comments_cursor = sync_state.as_ref()
        .and_then(|s| s.comments_last_sync.as_ref())
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc) - ChronoDuration::seconds(1));

    // Determine comments sync strategy:
    // - needs_full_sync: do full comments sync
    // - has cursor: do incremental sync
    // - in_cooldown with no cursor: skip comments (can't do anything safely)
    let comments_result = if needs_full_sync {
        match forge.list_all_comments(&repo).await {
            Ok(result) => result,
            Err(e) => {
                handle_rate_limit_error(&conn, &link.forge_type, forge.as_ref(), &e).await?;
                return Err(e);
            }
        }
    } else if let Some(cursor) = comments_cursor {
        match forge.list_comments_since(&repo, cursor).await {
            Ok(result) => result,
            Err(e) => {
                handle_rate_limit_error(&conn, &link.forge_type, forge.as_ref(), &e).await?;
                return Err(e);
            }
        }
    } else {
        // In cooldown with no cursor - skip comments sync
        FetchResult::complete(vec![])
    };

    let comments_stats = db::save_comments(
        &conn,
        &link.forge_repo,
        &comments_result.items,
        needs_full_sync,
        comments_result.is_complete,
    )?;

    // === GOALS (always full replace) ===
    if let Ok(goals) = forge.list_goals(&repo).await {
        let _ = db::save_goals(&conn, &link.forge_repo, &goals);
    }

    // Purge old tombstones during full sync
    if needs_full_sync {
        if let Ok((purged_issues, purged_comments)) = db::purge_deleted_items(&conn, 7) {
            if purged_issues > 0 || purged_comments > 0 {
                eprintln!(
                    "[daemon] Purged {} issue and {} comment tombstones for {}",
                    purged_issues, purged_comments, link.forge_repo
                );
            }
        }
    }

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
        "[daemon] {} sync for {}: {} issues (+{} -{} ~{}), {} comments (+{} -{} ~{})",
        if needs_full_sync { "Full" } else { "Incremental" },
        link.forge_repo,
        issues.len(),
        issues_stats.inserted,
        issues_stats.deleted,
        issues_stats.updated,
        comments_result.items.len(),
        comments_stats.inserted,
        comments_stats.deleted,
        comments_stats.updated,
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
                opts: std::collections::HashMap::new(),
            };
            let issue = forge.create_issue(repo, req).await?;
            let issue_display = crate::display::format_issue_id(&issue.id);
            eprintln!("[daemon] Created {} {}", issue_display, issue.title);
        }
        "comment" => {
            // Support both old issue_number and new issue_id keys
            let issue_id = payload["issue_id"].as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let body = payload["body"].as_str().unwrap_or("");
            forge.create_comment(repo, &issue_id, body).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Added comment to {}", issue_display);
        }
        "close" => {
            let issue_id = payload["issue_id"].as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            forge.close_issue(repo, &issue_id).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Closed {}", issue_display);
        }
        "reopen" => {
            let issue_id = payload["issue_id"].as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            forge.reopen_issue(repo, &issue_id).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Reopened {}", issue_display);
        }
        "label_add" => {
            let issue_id = payload["issue_id"].as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let label = payload["label"].as_str().unwrap_or("");
            forge.add_label(repo, &issue_id, label).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Added label '{}' to {}", label, issue_display);
        }
        "label_remove" => {
            let issue_id = payload["issue_id"].as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let label = payload["label"].as_str().unwrap_or("");
            forge.remove_label(repo, &issue_id, label).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Removed label '{}' from {}", label, issue_display);
        }
        "assign" => {
            let issue_id = payload["issue_id"].as_str()
                .map(|s| s.to_string())
                .or_else(|| payload["issue_number"].as_u64().map(|n| n.to_string()))
                .unwrap_or_default();
            let assignee = payload["assignee"].as_str().unwrap_or("");
            forge.assign_issue(repo, &issue_id, assignee).await?;
            let issue_display = crate::display::format_issue_id(&issue_id);
            eprintln!("[daemon] Assigned @{} to {}", assignee, issue_display);
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
        // 0 failures = base interval (15s) with jitter
        let backoff = calculate_backoff(0);
        let secs = backoff.as_secs_f64();

        // Base is 15s, jitter is ±25%, so range is 11.25 to 18.75
        assert!(secs >= 11.25, "backoff {} too low for 0 failures", secs);
        assert!(secs <= 18.75, "backoff {} too high for 0 failures", secs);
    }

    #[test]
    fn test_calculate_backoff_exponential_growth() {
        // Test that backoff grows exponentially (within jitter bounds)
        // 1 failure = 30s base, 2 = 60s, 3 = 120s, etc.

        let b1 = calculate_backoff(1);
        let b2 = calculate_backoff(2);
        let b3 = calculate_backoff(3);

        // With ±25% jitter: 1 failure = 22.5-37.5s, 2 = 45-75s, 3 = 90-150s
        assert!(b1.as_secs_f64() >= 22.5 && b1.as_secs_f64() <= 37.5,
            "1 failure backoff {} out of range", b1.as_secs_f64());
        assert!(b2.as_secs_f64() >= 45.0 && b2.as_secs_f64() <= 75.0,
            "2 failure backoff {} out of range", b2.as_secs_f64());
        assert!(b3.as_secs_f64() >= 90.0 && b3.as_secs_f64() <= 150.0,
            "3 failure backoff {} out of range", b3.as_secs_f64());
    }

    #[test]
    fn test_calculate_backoff_caps_at_max() {
        // Exponent caps at 6: 15 * 2^6 = 960s max
        // With ±25% jitter: 720 to 1200
        let backoff = calculate_backoff(10);
        let secs = backoff.as_secs_f64();

        assert!(secs >= 720.0, "max backoff {} too low", secs);
        assert!(secs <= 1200.0, "max backoff {} too high", secs);
    }

    #[test]
    fn test_calculate_backoff_very_high_failures() {
        // Even with extreme failures, should not overflow and should cap at 960s
        let backoff = calculate_backoff(100);
        let secs = backoff.as_secs_f64();

        // Should be capped at 960s with ±25% jitter = 720 to 1200
        assert!(secs >= 720.0 && secs <= 1200.0,
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

    #[tokio::test]
    async fn test_parallel_sync_executes_concurrently() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let c = Arc::clone(&counter);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Ok::<_, anyhow::Error>(())
                }
            })
            .collect();

        let start = Instant::now();
        let results: Vec<_> = stream::iter(tasks)
            .buffer_unordered(4)
            .collect()
            .await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 4);
        assert_eq!(counter.load(Ordering::SeqCst), 4);
        // Parallel: ~100ms. Sequential would be ~400ms.
        assert!(
            elapsed < Duration::from_millis(250),
            "took {:?}, expected parallel execution",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_backoff_state_updates_from_parallel_results() {
        let states: Arc<Mutex<HashMap<String, RepoSyncState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Simulate parallel sync results
        let results = vec![
            ("repo1".to_string(), SyncResult::Success),
            (
                "repo2".to_string(),
                SyncResult::Error(anyhow::anyhow!("network error")),
            ),
            ("repo3".to_string(), SyncResult::Skipped),
        ];

        let now = Instant::now();
        {
            let mut s = states.lock().await;
            for (repo, result) in results {
                match result {
                    SyncResult::Success => {
                        s.remove(&repo);
                    }
                    SyncResult::Skipped => {}
                    SyncResult::Error(_) => {
                        let state = s.entry(repo).or_insert(RepoSyncState {
                            consecutive_failures: 0,
                            next_attempt: now,
                        });
                        state.consecutive_failures += 1;
                        state.next_attempt = now + calculate_backoff(state.consecutive_failures);
                    }
                }
            }
        }

        let s = states.lock().await;
        assert!(!s.contains_key("repo1"), "success should remove backoff");
        assert!(s.contains_key("repo2"), "error should add backoff");
        assert_eq!(s.get("repo2").unwrap().consecutive_failures, 1);
        assert!(!s.contains_key("repo3"), "skipped should not modify state");
    }
}
