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

pub(crate) fn init_schema(conn: &Connection) -> Result<()> {
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
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS credentials (
            service TEXT PRIMARY KEY,
            access_token TEXT NOT NULL,
            refresh_token TEXT,
            expires_at TEXT
        );
        ",
    )?;

    // Migration: add display_name column if it doesn't exist
    // SQLite doesn't have IF NOT EXISTS for ALTER TABLE, so we check the schema
    let has_display_name: bool = conn
        .prepare("SELECT display_name FROM repo_links LIMIT 0")
        .is_ok();
    if !has_display_name {
        conn.execute("ALTER TABLE repo_links ADD COLUMN display_name TEXT", [])?;
    }

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
#[allow(dead_code)] // Used in tests
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

// === Watched Repos ===

/// A repo being watched by the daemon
#[derive(Debug, Clone)]
pub struct WatchedRepo {
    pub repo: String,
    pub last_accessed: String,
    pub added_at: String,
}

/// Add a repo to the watch list (or update if exists)
pub fn add_watched_repo(conn: &Connection, repo: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO watched_repos (repo, last_accessed, added_at)
         VALUES (?, datetime('now'), datetime('now'))
         ON CONFLICT(repo) DO UPDATE SET last_accessed = datetime('now')",
        params![repo],
    )?;
    Ok(())
}

/// Update last_accessed timestamp for a repo
pub fn touch_repo(conn: &Connection, repo: &str) -> Result<()> {
    let rows = conn.execute(
        "UPDATE watched_repos SET last_accessed = datetime('now') WHERE repo = ?",
        params![repo],
    )?;
    // If repo doesn't exist, add it
    if rows == 0 {
        add_watched_repo(conn, repo)?;
    }
    Ok(())
}

/// List all watched repos ordered by last_accessed (most recent first)
pub fn list_watched_repos(conn: &Connection) -> Result<Vec<WatchedRepo>> {
    let mut stmt = conn.prepare(
        "SELECT repo, last_accessed, added_at FROM watched_repos ORDER BY last_accessed DESC",
    )?;

    let repos = stmt
        .query_map([], |row| {
            Ok(WatchedRepo {
                repo: row.get(0)?,
                last_accessed: row.get(1)?,
                added_at: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(repos)
}

/// Remove a repo from the watch list
pub fn remove_watched_repo(conn: &Connection, repo: &str) -> Result<()> {
    conn.execute("DELETE FROM watched_repos WHERE repo = ?", params![repo])?;
    Ok(())
}

/// Clean up stale entries - removes watched_repos and repo_links for paths that no longer exist
pub fn cleanup_stale_repos(conn: &Connection) -> Result<usize> {
    let watched = list_watched_repos(conn)?;
    let mut removed = 0;

    for repo in watched {
        let path = std::path::Path::new(&repo.repo);
        // Remove if path doesn't exist or isn't a directory (valid git repo path)
        if !path.exists() || !path.is_dir() {
            conn.execute("DELETE FROM watched_repos WHERE repo = ?", params![repo.repo])?;
            conn.execute("DELETE FROM repo_links WHERE repo_path = ?", params![repo.repo])?;
            removed += 1;
        }
    }

    Ok(removed)
}

// === Repo Links ===

/// A link between a local git repo and its issue tracker (forge)
#[derive(Debug, Clone)]
pub struct RepoLink {
    pub repo_path: String,
    pub forge_type: String,
    pub forge_repo: String,
    pub display_name: Option<String>,
    pub created_at: String,
}

/// Get the link for a repo path
pub fn get_repo_link(conn: &Connection, repo_path: &str) -> Result<Option<RepoLink>> {
    let mut stmt = conn.prepare(
        "SELECT repo_path, forge_type, forge_repo, display_name, created_at FROM repo_links WHERE repo_path = ?",
    )?;

    let mut rows = stmt.query(params![repo_path])?;

    if let Some(row) = rows.next()? {
        Ok(Some(RepoLink {
            repo_path: row.get(0)?,
            forge_type: row.get(1)?,
            forge_repo: row.get(2)?,
            display_name: row.get(3)?,
            created_at: row.get(4)?,
        }))
    } else {
        Ok(None)
    }
}

/// Link a repo to a forge (insert or update)
pub fn set_repo_link(
    conn: &Connection,
    repo_path: &str,
    forge_type: &str,
    forge_repo: &str,
    display_name: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO repo_links (repo_path, forge_type, forge_repo, display_name, created_at)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(repo_path) DO UPDATE SET forge_type = ?, forge_repo = ?, display_name = ?",
        params![repo_path, forge_type, forge_repo, display_name, forge_type, forge_repo, display_name],
    )?;
    Ok(())
}

/// Remove the link for a repo
pub fn remove_repo_link(conn: &Connection, repo_path: &str) -> Result<()> {
    conn.execute("DELETE FROM repo_links WHERE repo_path = ?", params![repo_path])?;
    Ok(())
}

/// List all linked repos
pub fn list_repo_links(conn: &Connection) -> Result<Vec<RepoLink>> {
    let mut stmt = conn.prepare(
        "SELECT repo_path, forge_type, forge_repo, display_name, created_at FROM repo_links ORDER BY created_at DESC",
    )?;

    let links = stmt
        .query_map([], |row| {
            Ok(RepoLink {
                repo_path: row.get(0)?,
                forge_type: row.get(1)?,
                forge_repo: row.get(2)?,
                display_name: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(links)
}

// === Credentials ===

/// Stored OAuth credentials
#[derive(Debug, Clone)]
pub struct Credential {
    pub service: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<String>,
}

/// Get credentials for a service
pub fn get_credential(conn: &Connection, service: &str) -> Result<Option<Credential>> {
    let mut stmt = conn.prepare(
        "SELECT service, access_token, refresh_token, expires_at FROM credentials WHERE service = ?",
    )?;

    let mut rows = stmt.query(params![service])?;

    if let Some(row) = rows.next()? {
        Ok(Some(Credential {
            service: row.get(0)?,
            access_token: row.get(1)?,
            refresh_token: row.get(2)?,
            expires_at: row.get(3)?,
        }))
    } else {
        Ok(None)
    }
}

/// Save credentials for a service
pub fn set_credential(
    conn: &Connection,
    service: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO credentials (service, access_token, refresh_token, expires_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(service) DO UPDATE SET
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            expires_at = excluded.expires_at",
        params![service, access_token, refresh_token, expires_at],
    )?;
    Ok(())
}

/// Remove credentials for a service
pub fn remove_credential(conn: &Connection, service: &str) -> Result<()> {
    conn.execute("DELETE FROM credentials WHERE service = ?", params![service])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Issue, Label, User};

    /// Create an in-memory database for testing
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
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
        assert!(tables.contains(&"credentials".to_string()));
    }

    #[test]
    fn test_schema_is_idempotent() {
        let conn = test_db();
        // Running init_schema again should not error
        init_schema(&conn).unwrap();
        init_schema(&conn).unwrap();
    }

    // === Pending Ops Tests ===

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

    // === Issues Tests ===

    fn make_issue(number: u64, title: &str, state: &str, labels: Vec<&str>) -> Issue {
        Issue {
            number,
            title: title.to_string(),
            body: None,
            state: state.to_string(),
            user: User {
                login: "testuser".to_string(),
            },
            labels: labels
                .into_iter()
                .map(|name| Label {
                    name: name.to_string(),
                    color: "000000".to_string(),
                })
                .collect(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_save_and_load_issues() {
        let conn = test_db();

        let issues = vec![
            make_issue(1, "First", "open", vec![]),
            make_issue(2, "Second", "open", vec!["bug"]),
        ];

        save_issues(&conn, "owner/repo", &issues).unwrap();

        let loaded = load_issues(&conn, "owner/repo").unwrap();
        assert_eq!(loaded.len(), 2);
        // Ordered by number DESC
        assert_eq!(loaded[0].number, 2);
        assert_eq!(loaded[1].number, 1);
    }

    #[test]
    fn test_save_issues_replaces_existing() {
        let conn = test_db();

        save_issues(&conn, "owner/repo", &[make_issue(1, "Old", "open", vec![])]).unwrap();
        save_issues(&conn, "owner/repo", &[make_issue(2, "New", "open", vec![])]).unwrap();

        let loaded = load_issues(&conn, "owner/repo").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].title, "New");
    }

    #[test]
    fn test_filter_by_state() {
        let conn = test_db();

        let issues = vec![
            make_issue(1, "Open issue", "open", vec![]),
            make_issue(2, "Closed issue", "closed", vec![]),
        ];
        save_issues(&conn, "owner/repo", &issues).unwrap();

        let open = load_issues_filtered(&conn, "owner/repo", None, Some("open")).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].title, "Open issue");

        let closed = load_issues_filtered(&conn, "owner/repo", None, Some("closed")).unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].title, "Closed issue");
    }

    #[test]
    fn test_filter_by_label() {
        let conn = test_db();

        let issues = vec![
            make_issue(1, "Bug", "open", vec!["bug"]),
            make_issue(2, "Feature", "open", vec!["enhancement"]),
            make_issue(3, "Bug and feature", "open", vec!["bug", "enhancement"]),
        ];
        save_issues(&conn, "owner/repo", &issues).unwrap();

        let bugs = load_issues_filtered(&conn, "owner/repo", Some("bug"), None).unwrap();
        assert_eq!(bugs.len(), 2);

        let enhancements =
            load_issues_filtered(&conn, "owner/repo", Some("enhancement"), None).unwrap();
        assert_eq!(enhancements.len(), 2);
    }

    #[test]
    fn test_load_single_issue() {
        let conn = test_db();

        save_issues(
            &conn,
            "owner/repo",
            &[make_issue(42, "The answer", "open", vec![])],
        )
        .unwrap();

        let issue = load_issue(&conn, "owner/repo", 42).unwrap();
        assert!(issue.is_some());
        assert_eq!(issue.unwrap().title, "The answer");

        let missing = load_issue(&conn, "owner/repo", 999).unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_sync_state() {
        let conn = test_db();

        // No sync state initially
        assert!(get_sync_state(&conn, "owner/repo").unwrap().is_none());

        // After saving issues, sync state is recorded
        save_issues(&conn, "owner/repo", &[make_issue(1, "Test", "open", vec![])]).unwrap();

        let state = get_sync_state(&conn, "owner/repo").unwrap();
        assert!(state.is_some());
        let (_, count) = state.unwrap();
        assert_eq!(count, 1);
    }

    // === Watched Repos Tests ===

    #[test]
    fn test_add_watched_repo() {
        let conn = test_db();

        add_watched_repo(&conn, "owner/repo").unwrap();

        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].repo, "owner/repo");
    }

    #[test]
    fn test_add_watched_repo_is_idempotent() {
        let conn = test_db();

        add_watched_repo(&conn, "owner/repo").unwrap();
        add_watched_repo(&conn, "owner/repo").unwrap();

        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos.len(), 1);
    }

    #[test]
    fn test_touch_repo_adds_if_not_exists() {
        let conn = test_db();

        touch_repo(&conn, "owner/repo").unwrap();

        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].repo, "owner/repo");
    }

    #[test]
    fn test_touch_repo_updates_last_accessed() {
        let conn = test_db();

        add_watched_repo(&conn, "owner/repo").unwrap();
        let repos_before = list_watched_repos(&conn).unwrap();
        let accessed_before = repos_before[0].last_accessed.clone();

        // Small delay to ensure timestamp changes
        std::thread::sleep(std::time::Duration::from_millis(1100));
        touch_repo(&conn, "owner/repo").unwrap();

        let repos_after = list_watched_repos(&conn).unwrap();
        assert!(repos_after[0].last_accessed > accessed_before);
    }

    #[test]
    fn test_list_watched_repos_ordered_by_last_accessed() {
        let conn = test_db();

        add_watched_repo(&conn, "old/repo").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        add_watched_repo(&conn, "new/repo").unwrap();

        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos.len(), 2);
        // Most recently accessed first
        assert_eq!(repos[0].repo, "new/repo");
        assert_eq!(repos[1].repo, "old/repo");
    }

    #[test]
    fn test_remove_watched_repo() {
        let conn = test_db();

        add_watched_repo(&conn, "owner/repo").unwrap();
        add_watched_repo(&conn, "other/repo").unwrap();

        remove_watched_repo(&conn, "owner/repo").unwrap();

        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].repo, "other/repo");
    }

    #[test]
    fn test_remove_watched_repo_nonexistent() {
        let conn = test_db();

        // Should not error
        remove_watched_repo(&conn, "nonexistent/repo").unwrap();
    }

    // === Repo Links Tests ===

    #[test]
    fn test_set_and_get_repo_link() {
        let conn = test_db();

        set_repo_link(&conn, "/path/to/repo", "github", "owner/repo").unwrap();

        let link = get_repo_link(&conn, "/path/to/repo").unwrap();
        assert!(link.is_some());
        let link = link.unwrap();
        assert_eq!(link.repo_path, "/path/to/repo");
        assert_eq!(link.forge_type, "github");
        assert_eq!(link.forge_repo, "owner/repo");
    }

    #[test]
    fn test_get_repo_link_not_found() {
        let conn = test_db();

        let link = get_repo_link(&conn, "/nonexistent/path").unwrap();
        assert!(link.is_none());
    }

    #[test]
    fn test_set_repo_link_updates_existing() {
        let conn = test_db();

        set_repo_link(&conn, "/path/to/repo", "github", "owner/repo").unwrap();
        set_repo_link(&conn, "/path/to/repo", "linear", "team-id").unwrap();

        let link = get_repo_link(&conn, "/path/to/repo").unwrap().unwrap();
        assert_eq!(link.forge_type, "linear");
        assert_eq!(link.forge_repo, "team-id");
    }

    #[test]
    fn test_remove_repo_link() {
        let conn = test_db();

        set_repo_link(&conn, "/path/to/repo", "github", "owner/repo").unwrap();
        remove_repo_link(&conn, "/path/to/repo").unwrap();

        let link = get_repo_link(&conn, "/path/to/repo").unwrap();
        assert!(link.is_none());
    }

    #[test]
    fn test_remove_repo_link_nonexistent() {
        let conn = test_db();

        // Should not error
        remove_repo_link(&conn, "/nonexistent/path").unwrap();
    }

    #[test]
    fn test_list_repo_links() {
        let conn = test_db();

        set_repo_link(&conn, "/path/a", "github", "owner/a").unwrap();
        set_repo_link(&conn, "/path/b", "linear", "team-b").unwrap();

        let links = list_repo_links(&conn).unwrap();
        assert_eq!(links.len(), 2);
    }

    #[test]
    fn test_list_repo_links_empty() {
        let conn = test_db();

        let links = list_repo_links(&conn).unwrap();
        assert!(links.is_empty());
    }
}
