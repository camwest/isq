//! Database schema versioning and migrations
//!
//! # Adding a new migration
//!
//! 1. Add your migration SQL to the `MIGRATIONS` array
//! 2. Increment `CURRENT_VERSION`
//! 3. Run tests to verify
//!
//! ```ignore
//! // Example: adding a new column
//! const MIGRATIONS: &[&str] = &[
//!     "", // v1: baseline (no-op, handled separately)
//!     "ALTER TABLE issues ADD COLUMN my_new_column TEXT;",  // v2
//! ];
//! const CURRENT_VERSION: i64 = 2;
//! ```
//!
//! Migrations run sequentially. Each migration brings the schema from version N-1 to N.
//! Fresh installs get `SCHEMA_V1` directly, then run migrations 2+.

use anyhow::{Context, Result};
use rusqlite::Connection;

// ============================================================================
// SCHEMA VERSION - increment when adding migrations
// ============================================================================

const CURRENT_VERSION: i64 = 1;

// ============================================================================
// MIGRATIONS - add new migrations here
// ============================================================================

/// Migrations array. Index 0 is unused (v1 is baseline).
/// Each entry is SQL that migrates from version N-1 to N.
const MIGRATIONS: &[&str] = &[
    "", // v1: baseline schema, handled by SCHEMA_V1
       // v2 example (uncomment and modify when needed):
       // "ALTER TABLE issues ADD COLUMN some_new_field TEXT;",
];

// ============================================================================
// BASELINE SCHEMA (v1) - update when adding new tables
// ============================================================================

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS issues (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    issue_id TEXT,
    title TEXT NOT NULL,
    body TEXT,
    state TEXT NOT NULL,
    author TEXT NOT NULL,
    labels TEXT NOT NULL,
    assignees TEXT NOT NULL DEFAULT '[]',
    priority INTEGER NOT NULL DEFAULT 4,
    priority_label TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    html_url TEXT,
    milestone TEXT,
    parent_id TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_issues_repo ON issues(repo);
CREATE INDEX IF NOT EXISTS idx_issues_repo_number ON issues(repo, number);
CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_repo_issue_id ON issues(repo, issue_id);
CREATE INDEX IF NOT EXISTS idx_issues_parent_id ON issues(repo, parent_id);

CREATE TABLE IF NOT EXISTS sync_state (
    repo TEXT PRIMARY KEY,
    last_sync TEXT NOT NULL,
    issue_count INTEGER NOT NULL,
    issues_last_sync TEXT,
    comments_last_sync TEXT,
    goals_last_sync TEXT,
    last_full_sync_at TEXT,
    last_full_sync_attempt_at TEXT
);

CREATE TABLE IF NOT EXISTS pending_ops (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL,
    op_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_ops_repo ON pending_ops(repo);

CREATE TABLE IF NOT EXISTS watched_repos (
    repo TEXT PRIMARY KEY,
    last_accessed TEXT NOT NULL,
    added_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repo_links (
    repo_path TEXT PRIMARY KEY,
    forge_type TEXT NOT NULL,
    forge_repo TEXT NOT NULL,
    display_name TEXT,
    user_id TEXT,
    user_name TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS comments (
    id INTEGER PRIMARY KEY,
    forge_repo TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    issue_id TEXT,
    comment_id TEXT NOT NULL,
    body TEXT NOT NULL,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at TEXT,
    UNIQUE(forge_repo, comment_id)
);

CREATE INDEX IF NOT EXISTS idx_comments_issue ON comments(forge_repo, issue_number);
CREATE INDEX IF NOT EXISTS idx_comments_issue_id ON comments(forge_repo, issue_id);

CREATE TABLE IF NOT EXISTS goals (
    id INTEGER PRIMARY KEY,
    forge_repo TEXT NOT NULL,
    goal_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    target_date TEXT,
    state TEXT NOT NULL,
    open_count INTEGER DEFAULT 0,
    closed_count INTEGER DEFAULT 0,
    progress REAL DEFAULT 0.0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    html_url TEXT,
    UNIQUE(forge_repo, goal_id)
);

CREATE INDEX IF NOT EXISTS idx_goals_repo ON goals(forge_repo);
CREATE INDEX IF NOT EXISTS idx_goals_state ON goals(forge_repo, state);

CREATE TABLE IF NOT EXISTS rate_limit_state (
    forge TEXT PRIMARY KEY,
    rate_limit INTEGER,
    remaining INTEGER,
    reset_at INTEGER,
    last_error TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worktree_issues (
    git_dir TEXT NOT NULL,
    repo TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    issue_id TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (git_dir, issue_number)
);

CREATE INDEX IF NOT EXISTS idx_worktree_issues_git_dir ON worktree_issues(git_dir);

CREATE TABLE IF NOT EXISTS sync_stats (
    id INTEGER PRIMARY KEY,
    repo TEXT NOT NULL,
    data_type TEXT NOT NULL,
    sync_type TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    duration_ms INTEGER,
    items_fetched INTEGER,
    items_inserted INTEGER,
    items_updated INTEGER,
    items_deleted INTEGER,
    is_complete INTEGER,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_sync_stats_repo ON sync_stats(repo);
CREATE INDEX IF NOT EXISTS idx_sync_stats_started ON sync_stats(started_at);
";

// ============================================================================
// MIGRATION RUNNER - rarely needs modification
// ============================================================================

/// Initialize database schema and run any pending migrations.
pub(crate) fn init_schema(conn: &Connection) -> Result<()> {
    let db_version = get_user_version(conn)?;

    // Database from a newer isq version
    if db_version > CURRENT_VERSION {
        anyhow::bail!(
            "Database schema version ({db_version}) is newer than this version of isq \
             supports ({CURRENT_VERSION}). Please update isq to the latest version."
        );
    }

    // Legacy database (pre-versioning): migrate to v1
    if db_version == 0 && has_existing_tables(conn)? {
        migrate_legacy_to_v1(conn)?;
        return Ok(());
    }

    // Fresh install: create baseline schema
    if db_version == 0 {
        conn.execute_batch(SCHEMA_V1)
            .context("Failed to create database schema")?;
        set_user_version(conn, 1)?;
    }

    // Run any pending migrations (v2, v3, etc.)
    run_migrations(conn, db_version)?;

    Ok(())
}

/// Run migrations from current version to CURRENT_VERSION.
fn run_migrations(conn: &Connection, from_version: i64) -> Result<()> {
    for version in (from_version + 1)..=CURRENT_VERSION {
        let idx = version as usize;
        if idx < MIGRATIONS.len() && !MIGRATIONS[idx].is_empty() {
            conn.execute_batch(MIGRATIONS[idx])
                .with_context(|| format!("Migration to v{version} failed"))?;
        }
        set_user_version(conn, version)?;
    }
    Ok(())
}

fn get_user_version(conn: &Connection) -> Result<i64> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version)
}

fn set_user_version(conn: &Connection, version: i64) -> Result<()> {
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}

fn has_existing_tables(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='issues'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

// ============================================================================
// LEGACY MIGRATION - only for pre-versioning databases
// ============================================================================

/// Migrate a pre-versioning database to v1.
/// This code exists for backwards compatibility and should not be modified.
fn migrate_legacy_to_v1(conn: &Connection) -> Result<()> {
    let has_column = |table: &str, column: &str| -> bool {
        conn.prepare(&format!("SELECT {column} FROM {table} LIMIT 0"))
            .is_ok()
    };

    let add_column = |table: &str, column: &str, def: &str| -> Result<()> {
        if !has_column(table, column) {
            conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {def}"),
                [],
            )?;
        }
        Ok(())
    };

    // Issues table
    add_column("issues", "html_url", "TEXT")?;
    add_column("issues", "milestone", "TEXT")?;
    add_column("issues", "assignees", "TEXT NOT NULL DEFAULT '[]'")?;
    add_column("issues", "priority", "INTEGER NOT NULL DEFAULT 4")?;
    add_column("issues", "priority_label", "TEXT")?;
    add_column("issues", "deleted", "INTEGER NOT NULL DEFAULT 0")?;
    add_column("issues", "deleted_at", "TEXT")?;
    add_column("issues", "issue_id", "TEXT")?;

    if has_column("issues", "issue_id") {
        conn.execute(
            "UPDATE issues SET issue_id = CAST(number AS TEXT) WHERE issue_id IS NULL",
            [],
        )?;
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_issues_repo_issue_id ON issues(repo, issue_id)",
        [],
    )?;

    // Sync state
    add_column("sync_state", "issues_last_sync", "TEXT")?;
    add_column("sync_state", "comments_last_sync", "TEXT")?;
    add_column("sync_state", "goals_last_sync", "TEXT")?;
    add_column("sync_state", "last_full_sync_at", "TEXT")?;
    add_column("sync_state", "last_full_sync_attempt_at", "TEXT")?;

    // Repo links
    add_column("repo_links", "display_name", "TEXT")?;
    if !has_column("repo_links", "user_id") {
        if has_column("repo_links", "username") {
            conn.execute(
                "ALTER TABLE repo_links RENAME COLUMN username TO user_id",
                [],
            )?;
        } else {
            conn.execute("ALTER TABLE repo_links ADD COLUMN user_id TEXT", [])?;
        }
    }
    add_column("repo_links", "user_name", "TEXT")?;

    // Comments
    add_column("comments", "issue_id", "TEXT")?;
    add_column("comments", "updated_at", "TEXT")?;
    add_column("comments", "deleted", "INTEGER NOT NULL DEFAULT 0")?;
    add_column("comments", "deleted_at", "TEXT")?;

    if has_column("comments", "issue_id") {
        conn.execute(
            "UPDATE comments SET issue_id = CAST(issue_number AS TEXT) WHERE issue_id IS NULL",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_comments_issue_id ON comments(forge_repo, issue_id)",
            [],
        )?;
    }

    // Goals
    add_column("goals", "progress", "REAL DEFAULT 0.0")?;

    // Rate limit
    add_column("rate_limit_state", "rate_limit", "INTEGER")?;
    add_column("rate_limit_state", "remaining", "INTEGER")?;

    // Worktree issues
    add_column("worktree_issues", "issue_id", "TEXT")?;
    if has_column("worktree_issues", "issue_id") {
        conn.execute(
            "UPDATE worktree_issues SET issue_id = CAST(issue_number AS TEXT) WHERE issue_id IS NULL",
            [],
        )?;
    }

    // Remove old UNIQUE constraint on issues table
    let table_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='issues'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if table_sql.contains("UNIQUE(repo, number)") {
        conn.execute_batch(
            "
            CREATE TABLE issues_new (
                id INTEGER PRIMARY KEY,
                repo TEXT NOT NULL,
                number INTEGER NOT NULL,
                issue_id TEXT,
                title TEXT NOT NULL,
                body TEXT,
                state TEXT NOT NULL,
                author TEXT NOT NULL,
                labels TEXT NOT NULL,
                assignees TEXT NOT NULL DEFAULT '[]',
                priority INTEGER NOT NULL DEFAULT 4,
                priority_label TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                html_url TEXT,
                milestone TEXT,
                deleted INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT
            );
            INSERT INTO issues_new SELECT
                id, repo, number, issue_id, title, body, state, author,
                labels, assignees, priority, priority_label,
                created_at, updated_at, html_url, milestone, deleted, deleted_at
            FROM issues;
            DROP TABLE issues;
            ALTER TABLE issues_new RENAME TO issues;
            CREATE INDEX idx_issues_repo ON issues(repo);
            CREATE INDEX idx_issues_repo_number ON issues(repo, number);
            CREATE UNIQUE INDEX idx_issues_repo_issue_id ON issues(repo, issue_id);
            ",
        )?;
    }

    // Sync stats table
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sync_stats (
            id INTEGER PRIMARY KEY,
            repo TEXT NOT NULL,
            data_type TEXT NOT NULL,
            sync_type TEXT NOT NULL,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            duration_ms INTEGER,
            items_fetched INTEGER,
            items_inserted INTEGER,
            items_updated INTEGER,
            items_deleted INTEGER,
            is_complete INTEGER,
            error TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_sync_stats_repo ON sync_stats(repo);
        CREATE INDEX IF NOT EXISTS idx_sync_stats_started ON sync_stats(started_at);
        ",
    )?;

    // ========================================================================
    // Hierarchy support migrations
    // ========================================================================

    // Migration: add parent_id column to issues for hierarchy support
    let has_parent_id: bool = conn.prepare("SELECT parent_id FROM issues LIMIT 0").is_ok();
    if !has_parent_id {
        conn.execute("ALTER TABLE issues ADD COLUMN parent_id TEXT", [])?;
        // Create index for efficient child lookups
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_issues_parent_id ON issues(repo, parent_id)",
            [],
        )?;
    }

    set_user_version(conn, 1)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_install_sets_version() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn test_init_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
        assert_eq!(get_user_version(&conn).unwrap(), CURRENT_VERSION);
    }

    #[test]
    fn test_rejects_newer_database() {
        let conn = Connection::open_in_memory().unwrap();
        set_user_version(&conn, CURRENT_VERSION + 1).unwrap();
        let result = init_schema(&conn);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("newer"));
    }
}
