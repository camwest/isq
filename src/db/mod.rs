//! Database layer for SQLite cache
//!
//! This module provides persistence for issues, comments, goals, and sync state.
//! The database uses WAL mode for concurrent read/write access.

mod comments;
mod goals;
mod issues;
mod rate_limit;
mod repos;
pub(crate) mod schema;
mod sync_state;

use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

// Re-export public types and functions

// From comments
pub use comments::{Comment, count_comments_by_issue, load_comments, save_comments};

// From goals
pub use goals::{count_goals, load_goal_by_name, load_goals, save_goal, save_goals};

// From issues
pub use issues::{IssueFilter, load_issue, load_issues_with_filter, save_issues};

// From rate_limit
#[allow(unused_imports)]
pub use rate_limit::RateLimitState;
pub use rate_limit::{
    get_rate_limit_state, is_rate_limited, set_rate_limit_state, update_rate_limit_budget,
};

// From repos
#[allow(unused_imports)]
pub use repos::WatchedRepo;
pub use repos::{
    RepoLink, add_watched_repo, cleanup_stale_repos, clear_worktree_issues, get_repo_link,
    get_worktree_issue, list_watched_repos, remove_repo_link, remove_watched_repo, set_repo_link,
    set_worktree_issue, touch_repo,
};

// From sync_state
pub use sync_state::{
    PendingOp, SyncState, complete_op, count_pending_ops, get_sync_state, load_pending_ops,
    purge_deleted_items, queue_op,
};

/// Result of a sync operation with insert/update/delete counts
#[derive(Debug, Clone, Default)]
pub struct SyncResult {
    pub inserted: usize,
    pub updated: usize,
    pub deleted: usize,
}

/// Get the cache database path
pub fn db_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;

    let cache_dir = dirs.cache_dir();
    std::fs::create_dir_all(cache_dir)?;

    Ok(cache_dir.join("cache.db"))
}

/// Open database connection with WAL mode
pub fn open() -> Result<Connection> {
    let path = db_path()?;
    let conn = Connection::open(&path)?;

    // Enable WAL mode for concurrent read/write
    conn.pragma_update(None, "journal_mode", "WAL")?;

    // Initialize schema
    schema::init_schema(&conn)?;

    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::init_schema;
    use crate::forges::{Issue, Label};

    /// Create an in-memory database for testing
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    fn make_issue(id: &str, title: &str, state: &str, labels: Vec<&str>) -> Issue {
        Issue {
            id: id.to_string(),
            title: title.to_string(),
            body: None,
            state: state.to_string(),
            author: "testuser".to_string(),
            labels: labels
                .into_iter()
                .map(|s| Label::name_only(s.to_string()))
                .collect(),
            assignees: vec![],
            priority: 4,
            priority_label: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            url: None,
            milestone: None,
        }
    }

    // === Schema Tests ===

    #[test]
    fn test_schema_creates_all_tables() {
        let conn = test_db();

        // Verify all tables exist by querying sqlite_master
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"issues".to_string()));
        assert!(tables.contains(&"sync_state".to_string()));
        assert!(tables.contains(&"pending_ops".to_string()));
        assert!(tables.contains(&"watched_repos".to_string()));
        assert!(tables.contains(&"repo_links".to_string()));
        assert!(tables.contains(&"comments".to_string()));
        assert!(tables.contains(&"worktree_issues".to_string()));
    }

    #[test]
    fn test_schema_is_idempotent() {
        let conn = test_db();
        // Running init_schema again should not error
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
    }

    #[test]
    fn test_sync_state() {
        let conn = test_db();

        // No sync state initially
        assert!(get_sync_state(&conn, "owner/repo").unwrap().is_none());

        // After saving issues, sync state is recorded
        save_issues(
            &conn,
            "owner/repo",
            &[make_issue("1", "Test", "open", vec![])],
            true,
            true,
        )
        .unwrap();

        let state = get_sync_state(&conn, "owner/repo").unwrap();
        assert!(state.is_some());
        let sync_state = state.unwrap();
        assert_eq!(sync_state.issue_count, 1);
    }
}
