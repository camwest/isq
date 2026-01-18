//! Repository management - watched repos, repo links, and worktree issues

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

// ============================================================================
// Watched Repos
// ============================================================================

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
            conn.execute(
                "DELETE FROM repo_links WHERE repo_path = ?",
                params![repo.repo],
            )?;
            removed += 1;
        }
    }

    Ok(removed)
}

// ============================================================================
// Repo Links
// ============================================================================

/// A link between a local git repo and its issue tracker (forge)
#[derive(Debug, Clone)]
pub struct RepoLink {
    pub forge_type: String,
    pub forge_repo: String,
    pub display_name: Option<String>,
    /// The forge's native user identifier (GitHub username / Linear user UUID)
    pub user_id: Option<String>,
    /// User's display name for filtering (matches issue.assignees)
    pub user_name: Option<String>,
}

/// Get the link for a repo path
pub fn get_repo_link(conn: &Connection, repo_path: &str) -> Result<Option<RepoLink>> {
    let mut stmt = conn.prepare(
        "SELECT repo_path, forge_type, forge_repo, display_name, user_id, user_name, created_at FROM repo_links WHERE repo_path = ?",
    )?;

    let mut rows = stmt.query(params![repo_path])?;

    if let Some(row) = rows.next()? {
        Ok(Some(RepoLink {
            forge_type: row.get(1)?,
            forge_repo: row.get(2)?,
            display_name: row.get(3)?,
            user_id: row.get(4)?,
            user_name: row.get(5)?,
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
    user_id: Option<&str>,
    user_name: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO repo_links (repo_path, forge_type, forge_repo, display_name, user_id, user_name, created_at)
         VALUES (?, ?, ?, ?, ?, ?, datetime('now'))
         ON CONFLICT(repo_path) DO UPDATE SET forge_type = ?, forge_repo = ?, display_name = ?, user_id = ?, user_name = ?",
        params![repo_path, forge_type, forge_repo, display_name, user_id, user_name, forge_type, forge_repo, display_name, user_id, user_name],
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

// ============================================================================
// Worktree Issues
// ============================================================================

/// Set the current issue for a worktree (replaces any existing)
///
/// For v1, we enforce one issue per worktree by clearing existing associations first.
/// The schema supports multiple issues for future jj-style workflows.
pub fn set_worktree_issue(
    conn: &Connection,
    git_dir: &str,
    repo: &str,
    issue_id: &str,
) -> Result<()> {
    // Clear any existing associations for this worktree (v1: one issue per worktree)
    conn.execute(
        "DELETE FROM worktree_issues WHERE git_dir = ?",
        params![git_dir],
    )?;

    // Extract numeric part from issue ID for backward compatibility with 'issue_number' column
    // For "123" → 123, for "DEV-123" → 123
    let issue_number: i64 = issue_id
        .split('-')
        .last()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Insert the new association
    conn.execute(
        "INSERT INTO worktree_issues (git_dir, repo, issue_number, issue_id, created_at)
         VALUES (?, ?, ?, ?, datetime('now'))",
        params![git_dir, repo, issue_number, issue_id],
    )?;

    Ok(())
}

/// Get the current issue for a worktree
///
/// Returns (repo, issue_id) if an association exists.
pub fn get_worktree_issue(conn: &Connection, git_dir: &str) -> Result<Option<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT repo, issue_id FROM worktree_issues WHERE git_dir = ? LIMIT 1")?;

    let result = stmt
        .query_row(params![git_dir], |row| {
            let repo: String = row.get(0)?;
            let issue_id: String = row.get(1)?;
            Ok((repo, issue_id))
        })
        .optional()?;

    Ok(result)
}

/// Clear all issue associations for a worktree
pub fn clear_worktree_issues(conn: &Connection, git_dir: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM worktree_issues WHERE git_dir = ?",
        params![git_dir],
    )?;
    Ok(())
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
    fn test_touch_repo_updates_ordering() {
        let conn = test_db();

        // Add two repos
        add_watched_repo(&conn, "first/repo").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        add_watched_repo(&conn, "second/repo").unwrap();

        // Second repo is most recent
        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos[0].repo, "second/repo");

        // Touch first repo to make it most recent
        std::thread::sleep(std::time::Duration::from_millis(1100));
        touch_repo(&conn, "first/repo").unwrap();

        // First repo is now most recent
        let repos = list_watched_repos(&conn).unwrap();
        assert_eq!(repos[0].repo, "first/repo");
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

        set_repo_link(
            &conn,
            "/path/to/repo",
            "github",
            "owner/repo",
            None,
            None,
            None,
        )
        .unwrap();

        let link = get_repo_link(&conn, "/path/to/repo").unwrap();
        assert!(link.is_some());
        let link = link.unwrap();
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

        set_repo_link(
            &conn,
            "/path/to/repo",
            "github",
            "owner/repo",
            None,
            None,
            None,
        )
        .unwrap();
        set_repo_link(
            &conn,
            "/path/to/repo",
            "linear",
            "team-id",
            None,
            None,
            None,
        )
        .unwrap();

        let link = get_repo_link(&conn, "/path/to/repo").unwrap().unwrap();
        assert_eq!(link.forge_type, "linear");
        assert_eq!(link.forge_repo, "team-id");
    }

    #[test]
    fn test_remove_repo_link() {
        let conn = test_db();

        set_repo_link(
            &conn,
            "/path/to/repo",
            "github",
            "owner/repo",
            None,
            None,
            None,
        )
        .unwrap();
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
    fn test_repo_link_with_user_id_and_name() {
        let conn = test_db();

        set_repo_link(
            &conn,
            "/path/to/repo",
            "github",
            "owner/repo",
            None,
            Some("user-id-123"),
            Some("testuser"),
        )
        .unwrap();

        let link = get_repo_link(&conn, "/path/to/repo").unwrap().unwrap();
        assert_eq!(link.user_id, Some("user-id-123".to_string()));
        assert_eq!(link.user_name, Some("testuser".to_string()));
    }

    // === Worktree Issues Tests ===

    #[test]
    fn test_set_and_get_worktree_issue() {
        let conn = test_db();

        // Initially no issue associated
        let result = get_worktree_issue(&conn, "/path/to/.git").unwrap();
        assert!(result.is_none());

        // Set an issue
        set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "123").unwrap();

        // Get the issue back
        let result = get_worktree_issue(&conn, "/path/to/.git").unwrap();
        assert!(result.is_some());
        let (repo, issue_id) = result.unwrap();
        assert_eq!(repo, "owner/repo");
        assert_eq!(issue_id, "123");
    }

    #[test]
    fn test_worktree_issue_replaces_existing() {
        let conn = test_db();

        // Set first issue
        set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "100").unwrap();

        // Set a different issue (should replace)
        set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "200").unwrap();

        // Should get the new issue
        let result = get_worktree_issue(&conn, "/path/to/.git").unwrap().unwrap();
        assert_eq!(result.1, "200");
    }

    #[test]
    fn test_clear_worktree_issues() {
        let conn = test_db();

        // Set an issue
        set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "123").unwrap();

        // Verify it exists
        assert!(
            get_worktree_issue(&conn, "/path/to/.git")
                .unwrap()
                .is_some()
        );

        // Clear it
        clear_worktree_issues(&conn, "/path/to/.git").unwrap();

        // Verify it's gone
        assert!(
            get_worktree_issue(&conn, "/path/to/.git")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn test_worktree_issues_isolated_by_git_dir() {
        let conn = test_db();

        // Set issues for different worktrees
        set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "100").unwrap();
        set_worktree_issue(
            &conn,
            "/path/to/.git/worktrees/feature",
            "owner/repo",
            "DEV-200",
        )
        .unwrap();

        // Each worktree should have its own issue
        let main = get_worktree_issue(&conn, "/path/to/.git").unwrap().unwrap();
        let feature = get_worktree_issue(&conn, "/path/to/.git/worktrees/feature")
            .unwrap()
            .unwrap();

        assert_eq!(main.1, "100");
        assert_eq!(feature.1, "DEV-200");
    }
}
