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
    let mut stmt = conn.prepare(
        "SELECT number, title, body, state, author, labels, created_at, updated_at
         FROM issues WHERE repo = ? ORDER BY number DESC",
    )?;

    let issues = stmt
        .query_map(params![repo], |row| {
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
