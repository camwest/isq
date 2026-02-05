//! Worktree issues - associating issues with git worktrees

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

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
        .next_back()
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

/// Get all issue IDs that have active worktrees for a given repo
pub fn get_worktree_issue_ids(conn: &Connection, repo: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT issue_id FROM worktree_issues WHERE repo = ?")?;
    let ids = stmt
        .query_map(params![repo], |row| row.get::<_, String>(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(ids)
}
