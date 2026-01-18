//! Watched repos - repos being synced by the daemon

use anyhow::Result;
use rusqlite::{Connection, params};

use super::links::get_all_repo_link_paths;

/// A repo being watched by the daemon
#[derive(Debug, Clone)]
pub struct WatchedRepo {
    pub repo: String,
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
        .query_map([], |row| Ok(WatchedRepo { repo: row.get(0)? }))?
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
            conn.execute(
                "DELETE FROM watched_repos WHERE repo = ?",
                params![repo.repo],
            )?;
            removed += 1;
        }
    }

    // Also clean up repo_links for paths that no longer exist
    for repo_path in get_all_repo_link_paths(conn)? {
        let path = std::path::Path::new(&repo_path);
        if !path.exists() || !path.is_dir() {
            conn.execute(
                "DELETE FROM repo_links WHERE repo_path = ?",
                params![repo_path],
            )?;
            removed += 1;
        }
    }

    Ok(removed)
}
