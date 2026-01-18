//! Issue CRUD operations

mod filter;

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::forges::{Issue, Label};

use super::SyncResult;

#[cfg(test)]
mod tests;

// Re-export filter types and functions
#[cfg(test)]
pub(crate) use filter::parse_duration_to_sqlite_modifier;
pub use filter::{IssueFilter, load_issues_filtered, load_issues_with_filter};

/// Parse labels JSON with backward compatibility.
/// Handles both new format ([{"name": "bug", "color": "fc2929"}]) and old format (["bug"]).
pub(crate) fn parse_labels_json(json: &str) -> Vec<Label> {
    // Try new format first (Vec<Label>)
    if let Ok(labels) = serde_json::from_str::<Vec<Label>>(json) {
        return labels;
    }
    // Fall back to old format (Vec<String>)
    if let Ok(names) = serde_json::from_str::<Vec<String>>(json) {
        return names.into_iter().map(Label::name_only).collect();
    }
    Vec::new()
}

/// Save issues to database using UPSERT semantics for incremental sync
///
/// # Arguments
/// * `conn` - Database connection
/// * `repo` - Repository identifier (e.g., "owner/repo")
/// * `issues` - Issues to save
/// * `full_sync` - Whether this is a full sync (enables deletion reconciliation)
/// * `is_complete` - Whether the fetch was complete (only run deletion if true)
///
/// # Returns
/// SyncResult with counts of inserted, updated, and deleted issues
pub fn save_issues(
    conn: &Connection,
    repo: &str,
    issues: &[Issue],
    full_sync: bool,
    is_complete: bool,
) -> Result<SyncResult> {
    let tx = conn.unchecked_transaction()?;

    let mut inserted = 0;
    let mut updated = 0;

    // UPSERT each issue
    // We check if the row exists first to accurately count inserts vs updates
    // (SQLite's RETURNING with a subquery in ON CONFLICT is complex, this is clearer)
    for issue in issues {
        let labels_json = serde_json::to_string(&issue.labels)?;
        let assignees_json = serde_json::to_string(&issue.assignees)?;

        // Check if issue exists
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM issues WHERE repo = ? AND issue_id = ?",
                params![repo, issue.id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        // Extract numeric part from issue ID for backward compatibility with 'number' column
        // For "123" → 123, for "DEV-123" → 123
        let number: i64 = issue
            .id
            .split('-')
            .next_back()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        // UPSERT the issue, clearing deleted flag if it was set
        tx.execute(
            "INSERT INTO issues (repo, number, issue_id, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone, deleted, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0, NULL)
             ON CONFLICT(repo, issue_id) DO UPDATE SET
                number = excluded.number,
                title = excluded.title,
                body = excluded.body,
                state = excluded.state,
                author = excluded.author,
                labels = excluded.labels,
                assignees = excluded.assignees,
                priority = excluded.priority,
                priority_label = excluded.priority_label,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                html_url = excluded.html_url,
                milestone = excluded.milestone,
                deleted = 0,
                deleted_at = NULL",
            params![
                repo,
                number,
                issue.id,
                issue.title,
                issue.body,
                issue.state,
                issue.author,
                labels_json,
                assignees_json,
                issue.priority as i64,
                issue.priority_label,
                issue.created_at,
                issue.updated_at,
                issue.url,
                issue.milestone,
            ],
        )?;

        if exists {
            updated += 1;
        } else {
            inserted += 1;
        }
    }

    // During full sync with complete fetch: mark missing issues as deleted
    let deleted = if full_sync && is_complete && !issues.is_empty() {
        // Create temp table for seen issue IDs
        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS seen_issues (issue_id TEXT PRIMARY KEY)",
            [],
        )?;
        tx.execute("DELETE FROM seen_issues", [])?;

        // Batch insert seen IDs (500 at a time to avoid SQL length limits)
        for chunk in issues.chunks(500) {
            let placeholders: String = chunk.iter().map(|_| "(?)").collect::<Vec<_>>().join(",");
            let sql = format!("INSERT INTO seen_issues (issue_id) VALUES {}", placeholders);
            let params: Vec<&str> = chunk.iter().map(|i| i.id.as_str()).collect();
            tx.execute(&sql, rusqlite::params_from_iter(params))?;
        }

        // Mark unseen issues as deleted
        tx.execute(
            "UPDATE issues SET deleted = 1, deleted_at = datetime('now')
             WHERE repo = ? AND deleted = 0
             AND issue_id NOT IN (SELECT issue_id FROM seen_issues)",
            params![repo],
        )?
    } else if full_sync && is_complete && issues.is_empty() {
        // Special case: if API returns empty and it's a complete fetch,
        // mark all issues as deleted
        tx.execute(
            "UPDATE issues SET deleted = 1, deleted_at = datetime('now')
             WHERE repo = ? AND deleted = 0",
            params![repo],
        )?
    } else {
        0
    };

    // Calculate max updated_at from issues (server-derived cursor)
    let max_updated_at = issues.iter().map(|i| &i.updated_at).max().cloned();

    // Recount non-deleted issues
    let issue_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM issues WHERE repo = ? AND deleted = 0",
        params![repo],
        |row| row.get(0),
    )?;

    // Update sync state with server-derived cursor
    let now = chrono::Utc::now().to_rfc3339();
    if full_sync {
        if is_complete {
            // Full sync succeeded: update both last_full_sync_at and last_full_sync_attempt_at
            let full_sync_cursor = max_updated_at.as_ref().unwrap_or(&now);
            tx.execute(
                "INSERT INTO sync_state (repo, last_sync, issue_count, issues_last_sync,
                                         last_full_sync_at, last_full_sync_attempt_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(repo) DO UPDATE SET
                    last_sync = ?2,
                    issue_count = ?3,
                    issues_last_sync = COALESCE(?4, issues_last_sync),
                    last_full_sync_at = ?5,
                    last_full_sync_attempt_at = ?6",
                params![
                    repo,
                    now,
                    issue_count,
                    max_updated_at,
                    full_sync_cursor,
                    now
                ],
            )?;
        } else {
            // Full sync incomplete: only update last_full_sync_attempt_at (not last_full_sync_at)
            // This allows cooldown to prevent retry storms
            tx.execute(
                "INSERT INTO sync_state (repo, last_sync, issue_count, issues_last_sync,
                                         last_full_sync_attempt_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(repo) DO UPDATE SET
                    last_sync = ?2,
                    issue_count = ?3,
                    issues_last_sync = COALESCE(?4, issues_last_sync),
                    last_full_sync_attempt_at = ?5",
                params![repo, now, issue_count, max_updated_at, now],
            )?;
        }
    } else if let Some(cursor) = &max_updated_at {
        // Incremental sync
        tx.execute(
            "INSERT INTO sync_state (repo, last_sync, issue_count, issues_last_sync)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(repo) DO UPDATE SET
                last_sync = ?2,
                issue_count = ?3,
                issues_last_sync = ?4",
            params![repo, now, issue_count, cursor],
        )?;
    } else {
        // No issues fetched, just update last_sync
        tx.execute(
            "INSERT INTO sync_state (repo, last_sync, issue_count)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(repo) DO UPDATE SET
                last_sync = ?2,
                issue_count = ?3",
            params![repo, now, issue_count],
        )?;
    }

    tx.commit()?;
    Ok(SyncResult {
        inserted,
        updated,
        deleted,
    })
}

/// Load all issues for a repo from cache
#[allow(dead_code)] // Used in tests
pub fn load_issues(conn: &Connection, repo: &str) -> Result<Vec<Issue>> {
    load_issues_filtered(conn, repo, None, None, None, None, false, None, "priority")
}

/// Load a single issue from cache (excludes deleted issues)
pub fn load_issue(conn: &Connection, repo: &str, issue_id: &str) -> Result<Option<Issue>> {
    let mut stmt = conn.prepare(
        "SELECT issue_id, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone
         FROM issues WHERE repo = ? AND issue_id = ? AND deleted = 0",
    )?;

    let mut rows = stmt.query(params![repo, issue_id])?;

    if let Some(row) = rows.next()? {
        let labels_json: String = row.get(5)?;
        let labels = parse_labels_json(&labels_json);
        let assignees_json: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
        let assignees: Vec<String> = serde_json::from_str(&assignees_json).unwrap_or_default();
        let priority: i64 = row.get::<_, Option<i64>>(7)?.unwrap_or(4);

        Ok(Some(Issue {
            id: row.get(0)?,
            title: row.get(1)?,
            body: row.get(2)?,
            state: row.get(3)?,
            author: row.get(4)?,
            labels,
            assignees,
            priority: priority as u8,
            priority_label: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            url: row.get(11)?,
            milestone: row.get(12)?,
        }))
    } else {
        Ok(None)
    }
}
