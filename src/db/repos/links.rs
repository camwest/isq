//! Repo links - mapping local git repos to their issue trackers (forges)

use anyhow::Result;
use rusqlite::{Connection, params};

/// A link between a local git repo and its issue tracker (forge)
#[derive(Debug, Clone)]
pub struct RepoLink {
    pub forge_type: String,
    pub forge_repo: String,
    /// Forge-specific auth scope used to select credentials (e.g., Linear org/user scope)
    pub auth_scope: Option<String>,
    pub display_name: Option<String>,
    /// The forge's native user identifier (GitHub username / Linear user UUID)
    pub user_id: Option<String>,
    /// User's display name for filtering (matches issue.assignees)
    pub user_name: Option<String>,
}

/// Get the link for a repo path
pub fn get_repo_link(conn: &Connection, repo_path: &str) -> Result<Option<RepoLink>> {
    let mut stmt = conn.prepare(
        "SELECT repo_path, forge_type, forge_repo, auth_scope, display_name, user_id, user_name, created_at FROM repo_links WHERE repo_path = ?",
    )?;

    let mut rows = stmt.query(params![repo_path])?;

    if let Some(row) = rows.next()? {
        Ok(Some(RepoLink {
            forge_type: row.get(1)?,
            forge_repo: row.get(2)?,
            auth_scope: row.get(3)?,
            display_name: row.get(4)?,
            user_id: row.get(5)?,
            user_name: row.get(6)?,
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
    auth_scope: Option<&str>,
    display_name: Option<&str>,
    user_id: Option<&str>,
    user_name: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO repo_links (repo_path, forge_type, forge_repo, auth_scope, display_name, user_id, user_name, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))
         ON CONFLICT(repo_path) DO UPDATE SET forge_type = ?, forge_repo = ?, auth_scope = ?, display_name = ?, user_id = ?, user_name = ?",
        params![
            repo_path,
            forge_type,
            forge_repo,
            auth_scope,
            display_name,
            user_id,
            user_name,
            forge_type,
            forge_repo,
            auth_scope,
            display_name,
            user_id,
            user_name
        ],
    )?;
    Ok(())
}

/// Remove the link for a repo
pub fn remove_repo_link(conn: &Connection, repo_path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM repo_links WHERE repo_path = ?",
        params![repo_path],
    )?;
    Ok(())
}

/// Get all repo link paths (used for cleanup)
pub(super) fn get_all_repo_link_paths(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT repo_path FROM repo_links")?;
    let paths = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(paths)
}
