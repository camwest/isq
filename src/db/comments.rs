//! Comment storage and retrieval

use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;

use super::SyncResult;

/// A comment on an issue
#[derive(Debug, Clone)]
pub struct Comment {
    pub comment_id: String,
    pub issue_number: u64,
    pub body: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: Option<String>,
}

/// Save comments to database using UPSERT semantics for incremental sync
///
/// # Arguments
/// * `conn` - Database connection
/// * `forge_repo` - Repository identifier
/// * `comments` - Comments to save
/// * `full_sync` - Whether this is a full sync (enables deletion reconciliation)
/// * `is_complete` - Whether the fetch was complete (only run deletion if true)
///
/// # Returns
/// SyncResult with counts of inserted, updated, and deleted comments
pub fn save_comments(
    conn: &Connection,
    forge_repo: &str,
    comments: &[Comment],
    full_sync: bool,
    is_complete: bool,
) -> Result<SyncResult> {
    let tx = conn.unchecked_transaction()?;

    let mut inserted = 0;
    let mut updated = 0;

    // UPSERT each comment
    for comment in comments {
        // Check if comment exists
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM comments WHERE forge_repo = ? AND comment_id = ?",
                params![forge_repo, comment.comment_id],
                |_| Ok(true),
            )
            .unwrap_or(false);

        // UPSERT the comment, clearing deleted flag if it was set
        tx.execute(
            "INSERT INTO comments (forge_repo, issue_number, comment_id, body, author, created_at, updated_at, deleted, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, NULL)
             ON CONFLICT(forge_repo, comment_id) DO UPDATE SET
                issue_number = excluded.issue_number,
                body = excluded.body,
                author = excluded.author,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                deleted = 0,
                deleted_at = NULL",
            params![
                forge_repo,
                comment.issue_number as i64,
                comment.comment_id,
                comment.body,
                comment.author,
                comment.created_at,
                comment.updated_at,
            ],
        )?;

        if exists {
            updated += 1;
        } else {
            inserted += 1;
        }
    }

    // During full sync with complete fetch: mark missing comments as deleted
    let deleted = if full_sync && is_complete && !comments.is_empty() {
        // Create temp table for seen comment IDs
        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS seen_comments (comment_id TEXT PRIMARY KEY)",
            [],
        )?;
        tx.execute("DELETE FROM seen_comments", [])?;

        // Batch insert seen comment IDs
        for chunk in comments.chunks(500) {
            let placeholders: String = chunk.iter().map(|_| "(?)").collect::<Vec<_>>().join(",");
            let sql = format!(
                "INSERT INTO seen_comments (comment_id) VALUES {}",
                placeholders
            );
            let params: Vec<&str> = chunk.iter().map(|c| c.comment_id.as_str()).collect();
            tx.execute(&sql, rusqlite::params_from_iter(params))?;
        }

        // Mark unseen comments as deleted
        let count = tx.execute(
            "UPDATE comments SET deleted = 1, deleted_at = datetime('now')
             WHERE forge_repo = ? AND deleted = 0
             AND comment_id NOT IN (SELECT comment_id FROM seen_comments)",
            params![forge_repo],
        )?;

        count
    } else if full_sync && is_complete && comments.is_empty() {
        // Special case: if API returns empty and it's a complete fetch,
        // mark all comments as deleted
        let count = tx.execute(
            "UPDATE comments SET deleted = 1, deleted_at = datetime('now')
             WHERE forge_repo = ? AND deleted = 0",
            params![forge_repo],
        )?;
        count
    } else {
        0
    };

    // Calculate max updated_at from comments (server-derived cursor)
    let max_updated_at = comments
        .iter()
        .filter_map(|c| c.updated_at.as_ref())
        .max()
        .cloned();

    // Update comments_last_sync cursor
    if let Some(cursor) = max_updated_at {
        tx.execute(
            "UPDATE sync_state SET comments_last_sync = ? WHERE repo = ?",
            params![cursor, forge_repo],
        )?;
    }

    tx.commit()?;
    Ok(SyncResult {
        inserted,
        updated,
        deleted,
    })
}

/// Load comments for a specific issue (excludes deleted comments)
pub fn load_comments(
    conn: &Connection,
    forge_repo: &str,
    issue_number: u64,
) -> Result<Vec<Comment>> {
    let mut stmt = conn.prepare(
        "SELECT comment_id, issue_number, body, author, created_at, updated_at
         FROM comments WHERE forge_repo = ? AND issue_number = ? AND deleted = 0
         ORDER BY created_at ASC",
    )?;

    let comments = stmt
        .query_map(params![forge_repo, issue_number as i64], |row| {
            let num: i64 = row.get(1)?;
            Ok(Comment {
                comment_id: row.get(0)?,
                issue_number: num as u64,
                body: row.get(2)?,
                author: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(comments)
}

/// Count comments for each issue in a repo (returns map of issue_number -> count)
/// Excludes deleted comments
pub fn count_comments_by_issue(
    conn: &Connection,
    forge_repo: &str,
) -> Result<HashMap<u64, usize>> {
    let mut stmt = conn.prepare(
        "SELECT issue_number, COUNT(*) FROM comments WHERE forge_repo = ? AND deleted = 0 GROUP BY issue_number",
    )?;

    let mut counts = HashMap::new();
    let rows = stmt.query_map(params![forge_repo], |row| {
        let num: i64 = row.get(0)?;
        let count: i64 = row.get(1)?;
        Ok((num as u64, count as usize))
    })?;

    for row in rows {
        let (num, count) = row?;
        counts.insert(num, count);
    }

    Ok(counts)
}
