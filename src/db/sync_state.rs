//! Sync state and pending operations management

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use crate::db::rate_limit::RateLimitState;

/// Sync health status for a repository
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncHealth {
    /// Sync is working normally
    Healthy,
    /// Sync has issues but data may still be usable
    Degraded { reason: String, guidance: String },
    /// Sync is failing
    Unhealthy { reason: String, guidance: String },
}

impl SyncHealth {
    /// Check if this health status indicates a problem
    #[allow(dead_code)] // Will be used for programmatic checks
    pub fn has_problem(&self) -> bool {
        !matches!(self, SyncHealth::Healthy)
    }
}

/// Sync state for a repository with per-type cursors
#[derive(Debug, Clone, Default)]
pub struct SyncState {
    /// Timestamp of last sync (legacy, kept for backward compat)
    pub last_sync: Option<String>,
    /// Issue count
    pub issue_count: i64,
    /// Per-type sync cursors (RFC3339 UTC timestamps)
    pub issues_last_sync: Option<String>,
    pub comments_last_sync: Option<String>,
    #[allow(dead_code)] // Will be used when goals incremental sync is implemented
    pub goals_last_sync: Option<String>,
    /// Last successful full reconciliation timestamp
    pub last_full_sync_at: Option<String>,
    /// Last full sync attempt timestamp (regardless of success)
    pub last_full_sync_attempt_at: Option<String>,
}

/// Get sync state for a repo
pub fn get_sync_state(conn: &Connection, repo: &str) -> Result<Option<SyncState>> {
    let mut stmt = conn.prepare(
        "SELECT last_sync, issue_count, issues_last_sync, comments_last_sync, goals_last_sync,
                last_full_sync_at, last_full_sync_attempt_at
         FROM sync_state WHERE repo = ?",
    )?;

    let mut rows = stmt.query(params![repo])?;

    if let Some(row) = rows.next()? {
        Ok(Some(SyncState {
            last_sync: row.get(0)?,
            issue_count: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            issues_last_sync: row.get(2)?,
            comments_last_sync: row.get(3)?,
            goals_last_sync: row.get(4)?,
            last_full_sync_at: row.get(5)?,
            last_full_sync_attempt_at: row.get(6)?,
        }))
    } else {
        Ok(None)
    }
}

/// A pending operation queued for later sync
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for status display and debugging
pub struct PendingOp {
    pub id: i64,
    pub repo: String,
    pub op_type: String,
    pub payload: String,
    pub created_at: String,
}

/// Queue a write operation for later sync (used when offline)
pub fn queue_op(conn: &Connection, repo: &str, op_type: &str, payload: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO pending_ops (repo, op_type, payload, created_at)
         VALUES (?, ?, ?, datetime('now'))",
        params![repo, op_type, payload],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Load all pending operations for a repo
pub fn load_pending_ops(conn: &Connection, repo: &str) -> Result<Vec<PendingOp>> {
    let mut stmt = conn.prepare(
        "SELECT id, repo, op_type, payload, created_at
         FROM pending_ops WHERE repo = ? ORDER BY id ASC",
    )?;

    let ops = stmt
        .query_map(params![repo], |row| {
            Ok(PendingOp {
                id: row.get(0)?,
                repo: row.get(1)?,
                op_type: row.get(2)?,
                payload: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ops)
}

/// Delete a pending operation after successful sync
pub fn complete_op(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM pending_ops WHERE id = ?", params![id])?;
    Ok(())
}

/// Count pending operations for a repo
pub fn count_pending_ops(conn: &Connection, repo: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_ops WHERE repo = ?",
        params![repo],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Purge deleted items older than TTL
///
/// Removes tombstoned issues and comments that have been deleted for longer
/// than the specified number of days.
///
/// # Returns
/// Tuple of (issues_purged, comments_purged)
pub fn purge_deleted_items(conn: &Connection, ttl_days: i64) -> Result<(usize, usize)> {
    let threshold = format!("-{} days", ttl_days);

    let issues = conn.execute(
        "DELETE FROM issues WHERE deleted = 1 AND deleted_at < datetime('now', ?)",
        params![threshold],
    )?;

    let comments = conn.execute(
        "DELETE FROM comments WHERE deleted = 1 AND deleted_at < datetime('now', ?)",
        params![threshold],
    )?;

    Ok((issues, comments))
}

/// Thresholds for sync health evaluation
const STALE_SYNC_MINUTES: i64 = 30;
const VERY_STALE_SYNC_MINUTES: i64 = 120;

/// Calculate sync health for a repository
pub fn calculate_sync_health(
    sync_state: Option<&SyncState>,
    rate_limit_state: Option<&RateLimitState>,
    daemon_running: bool,
) -> SyncHealth {
    let now = Utc::now();

    // Check rate limit first (most actionable)
    if let Some(rl) = rate_limit_state
        && let Some(reset_at) = rl.reset_at
    {
        let now_ts = now.timestamp();
        if now_ts < reset_at {
            let reset_time = DateTime::from_timestamp(reset_at, 0)
                .map(|dt| {
                    use chrono::Local;
                    let local: chrono::DateTime<Local> = dt.into();
                    local.format("%-I:%M %p").to_string()
                })
                .unwrap_or_else(|| format!("{}s", reset_at - now_ts));

            let reason = rl
                .last_error
                .clone()
                .unwrap_or_else(|| "Rate limited".to_string());

            return SyncHealth::Degraded {
                reason,
                guidance: format!(
                    "Sync paused until {}. Will resume automatically.",
                    reset_time
                ),
            };
        }
    }

    // Check daemon status
    if !daemon_running {
        return SyncHealth::Unhealthy {
            reason: "Daemon not running".to_string(),
            guidance: "Start with: isq daemon start".to_string(),
        };
    }

    // Check sync freshness
    let Some(state) = sync_state else {
        return SyncHealth::Unhealthy {
            reason: "Never synced".to_string(),
            guidance: "Run: isq sync".to_string(),
        };
    };

    // Use last successful full sync time if available, fall back to issues_last_sync
    let last_sync_str = state
        .last_full_sync_at
        .as_ref()
        .or(state.issues_last_sync.as_ref());

    let Some(last_sync_str) = last_sync_str else {
        return SyncHealth::Unhealthy {
            reason: "Never synced".to_string(),
            guidance: "Run: isq sync".to_string(),
        };
    };

    let Ok(last_sync) = DateTime::parse_from_rfc3339(last_sync_str) else {
        return SyncHealth::Healthy; // Can't parse, assume OK
    };

    let minutes_since_sync = (now - last_sync.to_utc()).num_minutes();

    if minutes_since_sync > VERY_STALE_SYNC_MINUTES {
        return SyncHealth::Unhealthy {
            reason: format!("Last sync was {}+ minutes ago", VERY_STALE_SYNC_MINUTES),
            guidance: "Check daemon: isq daemon status".to_string(),
        };
    }

    if minutes_since_sync > STALE_SYNC_MINUTES {
        return SyncHealth::Degraded {
            reason: format!("Last sync was {} minutes ago", minutes_since_sync),
            guidance: "Data may be stale. Check: isq daemon status".to_string(),
        };
    }

    SyncHealth::Healthy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::init_schema;

    /// Create an in-memory database for testing
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_queue_and_load_pending_ops() {
        let conn = test_db();

        let id = queue_op(&conn, "owner/repo", "create", r#"{"title":"test"}"#).unwrap();
        assert!(id > 0);

        let ops = load_pending_ops(&conn, "owner/repo").unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op_type, "create");
        assert_eq!(ops[0].payload, r#"{"title":"test"}"#);
    }

    #[test]
    fn test_pending_ops_ordered_by_id() {
        let conn = test_db();

        queue_op(&conn, "owner/repo", "create", "first").unwrap();
        queue_op(&conn, "owner/repo", "comment", "second").unwrap();
        queue_op(&conn, "owner/repo", "close", "third").unwrap();

        let ops = load_pending_ops(&conn, "owner/repo").unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0].op_type, "create");
        assert_eq!(ops[1].op_type, "comment");
        assert_eq!(ops[2].op_type, "close");
    }

    #[test]
    fn test_pending_ops_isolated_by_repo() {
        let conn = test_db();

        queue_op(&conn, "repo-a", "create", "a").unwrap();
        queue_op(&conn, "repo-b", "create", "b").unwrap();

        let ops_a = load_pending_ops(&conn, "repo-a").unwrap();
        let ops_b = load_pending_ops(&conn, "repo-b").unwrap();

        assert_eq!(ops_a.len(), 1);
        assert_eq!(ops_b.len(), 1);
        assert_eq!(ops_a[0].payload, "a");
        assert_eq!(ops_b[0].payload, "b");
    }

    #[test]
    fn test_complete_op_removes_from_queue() {
        let conn = test_db();

        let id1 = queue_op(&conn, "owner/repo", "create", "first").unwrap();
        let id2 = queue_op(&conn, "owner/repo", "comment", "second").unwrap();

        complete_op(&conn, id1).unwrap();

        let ops = load_pending_ops(&conn, "owner/repo").unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, id2);
    }

    #[test]
    fn test_count_pending_ops() {
        let conn = test_db();

        assert_eq!(count_pending_ops(&conn, "owner/repo").unwrap(), 0);

        queue_op(&conn, "owner/repo", "create", "1").unwrap();
        assert_eq!(count_pending_ops(&conn, "owner/repo").unwrap(), 1);

        queue_op(&conn, "owner/repo", "create", "2").unwrap();
        assert_eq!(count_pending_ops(&conn, "owner/repo").unwrap(), 2);

        queue_op(&conn, "other/repo", "create", "3").unwrap();
        assert_eq!(count_pending_ops(&conn, "owner/repo").unwrap(), 2);
        assert_eq!(count_pending_ops(&conn, "other/repo").unwrap(), 1);
    }
}
