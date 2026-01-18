//! Background sync daemon for isq.

mod process;
mod queue;
mod sync;

#[cfg(test)]
mod tests;

use anyhow::Result;
use chrono::Utc;
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::db;

// Re-export public items
pub use process::{DaemonInfo, read_daemon_info};
use process::{acquire_lock, write_daemon_info};
use sync::sync_once;

// Sync all repos at this interval
const SYNC_INTERVAL_SECS: u64 = 15; // Reduced from 30s since incremental is cheaper
const MAX_BACKOFF_SECS: u64 = 3600; // Max 1 hour backoff
const MAX_CONCURRENT_SYNCS: usize = 4; // Max repos to sync in parallel
const VERSION_CHECK_INTERVAL_SECS: u64 = 300; // 5 minutes - check if binary was updated

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
    if let Ok(conn) = db::open()
        && let Ok(removed) = db::cleanup_stale_repos(&conn)
        && removed > 0
    {
        eprintln!("[daemon] Cleaned up {} stale repo entries", removed);
    }

    // Track per-repo backoff state (thread-safe for parallel sync)
    let repo_states: Arc<Mutex<HashMap<String, RepoSyncState>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Create intervals for sync and version check
    let mut sync_interval = tokio::time::interval(Duration::from_secs(SYNC_INTERVAL_SECS));
    let mut version_interval =
        tokio::time::interval(Duration::from_secs(VERSION_CHECK_INTERVAL_SECS));

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
                    if let Some(state) = states.get(&repo_path)
                        && Instant::now() < state.next_attempt
                    {
                        return (repo_path, SyncResult::Skipped);
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

                    let state = states.entry(repo_path.clone()).or_insert(RepoSyncState {
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
