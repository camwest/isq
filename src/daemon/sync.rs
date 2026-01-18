//! Single-repo sync implementation.

use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};

use crate::db;
use crate::forges::{FetchResult, Forge, get_forge_for_repo};
use crate::repo::Repo;

use super::queue::process_pending_ops;

/// Minutes to wait after failed full sync attempt before retrying
const FULL_SYNC_RETRY_COOLDOWN_MINS: i64 = 15;

/// Hours between full reconciliation syncs
const FULL_SYNC_INTERVAL_HOURS: i64 = 1;

/// Determine if we need a full sync based on sync state
/// Returns (should_do_full_sync, in_cooldown)
pub fn should_do_full_sync(sync_state: &Option<db::SyncState>, has_cursor: bool) -> (bool, bool) {
    let Some(state) = sync_state else {
        return (true, false); // First sync ever
    };

    let now = Utc::now();

    // Check if we're in cooldown from a recent attempt (prevents retry storms)
    let in_cooldown = state
        .last_full_sync_attempt_at
        .as_ref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| {
            now - t.with_timezone(&Utc) <= ChronoDuration::minutes(FULL_SYNC_RETRY_COOLDOWN_MINS)
        })
        .unwrap_or(false);

    if in_cooldown {
        return (false, true);
    }

    // Not in cooldown - check if we need full sync
    if !has_cursor {
        return (true, false); // No cursor available, must do full sync
    }

    // Check if successful full sync is stale (> 1 hour)
    let needs_full = state
        .last_full_sync_at
        .as_ref()
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
pub async fn sync_once(repo_path: &str) -> Result<()> {
    // Look up the repo link to get forge info
    let (forge, link) = get_forge_for_repo(repo_path)?;

    let conn = db::open()?;

    // Check if we're rate limited for this forge
    if db::is_rate_limited(&conn, &link.forge_type)?
        && let Some(state) = db::get_rate_limit_state(&conn, &link.forge_type)?
        && let Some(reset_at) = state.reset_at
    {
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
        eprintln!(
            "[daemon] Processing {} pending operations...",
            pending_ops.len()
        );
        let synced = process_pending_ops(forge.as_ref(), &repo, &conn, &pending_ops).await;
        if synced > 0 {
            eprintln!("[daemon] Synced {} pending operations", synced);
        }
    }

    // === ISSUES ===
    // Calculate cursor for incremental sync (subtract 1 second for safety buffer)
    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
    let issues_cursor = sync_state
        .as_ref()
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
    let comments_cursor = sync_state
        .as_ref()
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
    if needs_full_sync
        && let Ok((purged_issues, purged_comments)) = db::purge_deleted_items(&conn, 7)
        && (purged_issues > 0 || purged_comments > 0)
    {
        eprintln!(
            "[daemon] Purged {} issue and {} comment tombstones for {}",
            purged_issues, purged_comments, link.forge_repo
        );
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
        if needs_full_sync {
            "Full"
        } else {
            "Incremental"
        },
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
