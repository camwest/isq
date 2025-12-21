use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::github::Issue;

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
    init_schema(&conn)?;

    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS issues (
            id INTEGER PRIMARY KEY,
            repo TEXT NOT NULL,
            number INTEGER NOT NULL,
            title TEXT NOT NULL,
            body TEXT,
            state TEXT NOT NULL,
            author TEXT NOT NULL,
            labels TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(repo, number)
        );

        CREATE INDEX IF NOT EXISTS idx_issues_repo ON issues(repo);
        CREATE INDEX IF NOT EXISTS idx_issues_repo_number ON issues(repo, number);

        CREATE TABLE IF NOT EXISTS sync_state (
            repo TEXT PRIMARY KEY,
            last_sync TEXT NOT NULL,
            issue_count INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pending_ops (
            id INTEGER PRIMARY KEY,
            repo TEXT NOT NULL,
            op_type TEXT NOT NULL,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_pending_ops_repo ON pending_ops(repo);
        ",
    )?;

    Ok(())
}

/// Save issues to database (full replace for a repo)
pub fn save_issues(conn: &Connection, repo: &str, issues: &[Issue]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;

    // Delete existing issues for this repo
    tx.execute("DELETE FROM issues WHERE repo = ?", params![repo])?;

    // Insert new issues
    let mut stmt = tx.prepare(
        "INSERT INTO issues (repo, number, title, body, state, author, labels, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;

    for issue in issues {
        let labels_json = serde_json::to_string(&issue.labels)?;
        stmt.execute(params![
            repo,
            issue.number as i64,
            issue.title,
            issue.body,
            issue.state,
            issue.user.login,
            labels_json,
            issue.created_at,
            issue.updated_at,
        ])?;
    }

    // Drop statement before committing
    drop(stmt);

    // Update sync state
    tx.execute(
        "INSERT OR REPLACE INTO sync_state (repo, last_sync, issue_count)
         VALUES (?, datetime('now'), ?)",
        params![repo, issues.len() as i64],
    )?;

    tx.commit()?;
    Ok(())
}

/// Load all issues for a repo from cache
pub fn load_issues(conn: &Connection, repo: &str) -> Result<Vec<Issue>> {
    load_issues_filtered(conn, repo, None, None)
}

/// Load issues with optional filters
pub fn load_issues_filtered(
    conn: &Connection,
    repo: &str,
    label: Option<&str>,
    state: Option<&str>,
) -> Result<Vec<Issue>> {
    // Build query dynamically based on filters
    let mut sql = String::from(
        "SELECT number, title, body, state, author, labels, created_at, updated_at
         FROM issues WHERE repo = ?",
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(repo.to_string())];

    if let Some(s) = state {
        sql.push_str(" AND state = ?");
        params_vec.push(Box::new(s.to_string()));
    }

    if let Some(l) = label {
        // Labels are stored as JSON array, search for label name
        sql.push_str(" AND labels LIKE ?");
        params_vec.push(Box::new(format!("%\"name\":\"{}\",%", l)));
    }

    sql.push_str(" ORDER BY number DESC");

    let mut stmt = conn.prepare(&sql)?;

    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let issues = stmt
        .query_map(params_refs.as_slice(), |row| {
            let number: i64 = row.get(0)?;
            let labels_json: String = row.get(5)?;
            let labels: Vec<crate::github::Label> =
                serde_json::from_str(&labels_json).unwrap_or_default();

            Ok(Issue {
                number: number as u64,
                title: row.get(1)?,
                body: row.get(2)?,
                state: row.get(3)?,
                user: crate::github::User {
                    login: row.get(4)?,
                },
                labels,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(issues)
}

/// Load a single issue from cache
pub fn load_issue(conn: &Connection, repo: &str, number: u64) -> Result<Option<Issue>> {
    let mut stmt = conn.prepare(
        "SELECT number, title, body, state, author, labels, created_at, updated_at
         FROM issues WHERE repo = ? AND number = ?",
    )?;

    let mut rows = stmt.query(params![repo, number as i64])?;

    if let Some(row) = rows.next()? {
        let num: i64 = row.get(0)?;
        let labels_json: String = row.get(5)?;
        let labels: Vec<crate::github::Label> =
            serde_json::from_str(&labels_json).unwrap_or_default();

        Ok(Some(Issue {
            number: num as u64,
            title: row.get(1)?,
            body: row.get(2)?,
            state: row.get(3)?,
            user: crate::github::User {
                login: row.get(4)?,
            },
            labels,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        }))
    } else {
        Ok(None)
    }
}

/// Get sync state for a repo
pub fn get_sync_state(conn: &Connection, repo: &str) -> Result<Option<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT last_sync, issue_count FROM sync_state WHERE repo = ?",
    )?;

    let mut rows = stmt.query(params![repo])?;

    if let Some(row) = rows.next()? {
        Ok(Some((row.get(0)?, row.get(1)?)))
    } else {
        Ok(None)
    }
}

/// A pending operation queued for later sync
#[derive(Debug, Clone)]
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
