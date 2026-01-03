//! Issue CRUD operations

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::forges::{Issue, Label};

use super::SyncResult;

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
                "SELECT 1 FROM issues WHERE repo = ? AND number = ?",
                params![repo, issue.number as i64],
                |_| Ok(true),
            )
            .unwrap_or(false);

        // UPSERT the issue, clearing deleted flag if it was set
        tx.execute(
            "INSERT INTO issues (repo, number, key, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone, deleted, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0, NULL)
             ON CONFLICT(repo, number) DO UPDATE SET
                key = excluded.key,
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
                issue.number as i64,
                issue.key,
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
        // Create temp table for seen issue numbers
        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS seen_issues (number INTEGER PRIMARY KEY)",
            [],
        )?;
        tx.execute("DELETE FROM seen_issues", [])?;

        // Batch insert seen numbers (500 at a time to avoid SQL length limits)
        for chunk in issues.chunks(500) {
            let placeholders: String = chunk.iter().map(|_| "(?)").collect::<Vec<_>>().join(",");
            let sql = format!(
                "INSERT INTO seen_issues (number) VALUES {}",
                placeholders
            );
            let params: Vec<i64> = chunk.iter().map(|i| i.number as i64).collect();
            tx.execute(&sql, rusqlite::params_from_iter(params))?;
        }

        // Mark unseen issues as deleted
        let count = tx.execute(
            "UPDATE issues SET deleted = 1, deleted_at = datetime('now')
             WHERE repo = ? AND deleted = 0
             AND number NOT IN (SELECT number FROM seen_issues)",
            params![repo],
        )?;

        count
    } else if full_sync && is_complete && issues.is_empty() {
        // Special case: if API returns empty and it's a complete fetch,
        // mark all issues as deleted
        let count = tx.execute(
            "UPDATE issues SET deleted = 1, deleted_at = datetime('now')
             WHERE repo = ? AND deleted = 0",
            params![repo],
        )?;
        count
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
    if full_sync && is_complete {
        // Use server-derived timestamp (max_updated_at) for last_full_sync_at
        // Fall back to client time if no issues were returned
        let full_sync_cursor = max_updated_at.as_ref().unwrap_or(&now);
        tx.execute(
            "INSERT INTO sync_state (repo, last_sync, issue_count, issues_last_sync, last_full_sync_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo) DO UPDATE SET
                last_sync = ?2,
                issue_count = ?3,
                issues_last_sync = COALESCE(?4, issues_last_sync),
                last_full_sync_at = ?5",
            params![repo, now, issue_count, max_updated_at, full_sync_cursor],
        )?;
    } else if let Some(cursor) = &max_updated_at {
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

/// Load issues with optional filters
pub fn load_issues_filtered(
    conn: &Connection,
    repo: &str,
    ids: Option<&[u64]>,
    label: Option<&str>,
    state: Option<&str>,
    assignee: Option<&str>,
    unassigned: bool,
    goal: Option<&str>,
    sort: &str,
) -> Result<Vec<Issue>> {
    // Build query dynamically based on filters
    // Always exclude deleted issues
    let mut sql = String::from(
        "SELECT number, key, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone
         FROM issues WHERE repo = ? AND deleted = 0",
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(repo.to_string())];

    // Filter by specific IDs if provided
    if let Some(id_list) = ids {
        if !id_list.is_empty() {
            let placeholders: Vec<&str> = id_list.iter().map(|_| "?").collect();
            sql.push_str(&format!(" AND number IN ({})", placeholders.join(",")));
            for id in id_list {
                params_vec.push(Box::new(*id as i64));
            }
        }
    }

    if let Some(s) = state {
        sql.push_str(" AND state = ?");
        params_vec.push(Box::new(s.to_string()));
    }

    if let Some(l) = label {
        // Labels are stored as JSON array of strings, e.g. ["bug","enhancement"]
        sql.push_str(" AND labels LIKE ?");
        params_vec.push(Box::new(format!("%\"{}\"%", l)));
    }

    if let Some(a) = assignee {
        // Assignees are stored as JSON array of strings, e.g. ["user1","user2"]
        sql.push_str(" AND assignees LIKE ?");
        params_vec.push(Box::new(format!("%\"{}\"%", a)));
    }

    if unassigned {
        // Unassigned issues have empty assignees array
        sql.push_str(" AND (assignees = '[]' OR assignees IS NULL OR assignees = '')");
    }

    if let Some(g) = goal {
        sql.push_str(" AND milestone = ?");
        params_vec.push(Box::new(g.to_string()));
    }

    // Apply sort order
    let order_by = match sort {
        "newest" => "number DESC",
        "oldest" => "number ASC",
        "updated" => "updated_at DESC",
        _ => "priority ASC, number DESC", // default: priority
    };
    sql.push_str(" ORDER BY ");
    sql.push_str(order_by);

    let mut stmt = conn.prepare(&sql)?;

    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let issues = stmt
        .query_map(params_refs.as_slice(), |row| {
            let number: i64 = row.get(0)?;
            let key: Option<String> = row.get(1)?;
            let labels_json: String = row.get(6)?;
            let labels = parse_labels_json(&labels_json);
            let assignees_json: String = row.get::<_, Option<String>>(7)?.unwrap_or_default();
            let assignees: Vec<String> =
                serde_json::from_str(&assignees_json).unwrap_or_default();
            let priority: i64 = row.get::<_, Option<i64>>(8)?.unwrap_or(4);

            Ok(Issue {
                number: number as u64,
                key,
                title: row.get(2)?,
                body: row.get(3)?,
                state: row.get(4)?,
                author: row.get(5)?,
                labels,
                assignees,
                priority: priority as u8,
                priority_label: row.get(9)?,
                created_at: row.get(10)?,
                updated_at: row.get(11)?,
                url: row.get(12)?,
                milestone: row.get(13)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(issues)
}

/// Load a single issue from cache (excludes deleted issues)
pub fn load_issue(conn: &Connection, repo: &str, number: u64) -> Result<Option<Issue>> {
    let mut stmt = conn.prepare(
        "SELECT number, key, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone
         FROM issues WHERE repo = ? AND number = ? AND deleted = 0",
    )?;

    let mut rows = stmt.query(params![repo, number as i64])?;

    if let Some(row) = rows.next()? {
        let num: i64 = row.get(0)?;
        let key: Option<String> = row.get(1)?;
        let labels_json: String = row.get(6)?;
        let labels = parse_labels_json(&labels_json);
        let assignees_json: String = row.get::<_, Option<String>>(7)?.unwrap_or_default();
        let assignees: Vec<String> = serde_json::from_str(&assignees_json).unwrap_or_default();
        let priority: i64 = row.get::<_, Option<i64>>(8)?.unwrap_or(4);

        Ok(Some(Issue {
            number: num as u64,
            key,
            title: row.get(2)?,
            body: row.get(3)?,
            state: row.get(4)?,
            author: row.get(5)?,
            labels,
            assignees,
            priority: priority as u8,
            priority_label: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            url: row.get(12)?,
            milestone: row.get(13)?,
        }))
    } else {
        Ok(None)
    }
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

    fn make_issue(number: u64, title: &str, state: &str, labels: Vec<&str>) -> Issue {
        Issue {
            number,
            key: None,
            title: title.to_string(),
            body: None,
            state: state.to_string(),
            author: "testuser".to_string(),
            labels: labels
                .into_iter()
                .map(|s| Label::name_only(s.to_string()))
                .collect(),
            assignees: vec![],
            priority: 4, // Default: none
            priority_label: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            url: None,
            milestone: None,
        }
    }

    // === Label Parsing Tests ===

    #[test]
    fn test_parse_labels_json_new_format() {
        let json = r##"[{"name":"bug","color":"#fc2929"},{"name":"feature","color":"#4EA7FC"}]"##;
        let labels = parse_labels_json(json);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[0].color, Some("#fc2929".to_string()));
        assert_eq!(labels[1].name, "feature");
        assert_eq!(labels[1].color, Some("#4EA7FC".to_string()));
    }

    #[test]
    fn test_parse_labels_json_old_format() {
        let json = r#"["bug","enhancement"]"#;
        let labels = parse_labels_json(json);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[0].color, None);
        assert_eq!(labels[1].name, "enhancement");
        assert_eq!(labels[1].color, None);
    }

    #[test]
    fn test_parse_labels_json_empty() {
        assert!(parse_labels_json("[]").is_empty());
        assert!(parse_labels_json("").is_empty());
        assert!(parse_labels_json("invalid").is_empty());
    }

    // === Issues Tests ===

    #[test]
    fn test_save_and_load_issues() {
        let conn = test_db();

        let issues = vec![
            make_issue(1, "First", "open", vec![]),
            make_issue(2, "Second", "open", vec!["bug"]),
        ];

        save_issues(&conn, "owner/repo", &issues, true, true).unwrap();

        let loaded = load_issues(&conn, "owner/repo").unwrap();
        assert_eq!(loaded.len(), 2);
        // Ordered by number DESC
        assert_eq!(loaded[0].number, 2);
        assert_eq!(loaded[1].number, 1);
    }

    #[test]
    fn test_save_issues_replaces_existing() {
        let conn = test_db();

        save_issues(
            &conn,
            "owner/repo",
            &[make_issue(1, "Old", "open", vec![])],
            true,
            true,
        )
        .unwrap();
        save_issues(
            &conn,
            "owner/repo",
            &[make_issue(2, "New", "open", vec![])],
            true,
            true,
        )
        .unwrap();

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
        save_issues(&conn, "owner/repo", &issues, true, true).unwrap();

        let open = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            Some("open"),
            None,
            false,
            None,
            "priority",
        )
        .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].title, "Open issue");

        let closed = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            Some("closed"),
            None,
            false,
            None,
            "priority",
        )
        .unwrap();
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
        save_issues(&conn, "owner/repo", &issues, true, true).unwrap();

        let bugs = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            Some("bug"),
            None,
            None,
            false,
            None,
            "priority",
        )
        .unwrap();
        assert_eq!(bugs.len(), 2);

        let enhancements = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            Some("enhancement"),
            None,
            None,
            false,
            None,
            "priority",
        )
        .unwrap();
        assert_eq!(enhancements.len(), 2);
    }

    #[test]
    fn test_load_single_issue() {
        let conn = test_db();

        save_issues(
            &conn,
            "owner/repo",
            &[make_issue(42, "The answer", "open", vec![])],
            true,
            true,
        )
        .unwrap();

        let issue = load_issue(&conn, "owner/repo", 42).unwrap();
        assert!(issue.is_some());
        assert_eq!(issue.unwrap().title, "The answer");

        let missing = load_issue(&conn, "owner/repo", 999).unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_filter_by_assignee() {
        let conn = test_db();
        let mut issue1 = make_issue(1, "Assigned to alice", "open", vec![]);
        issue1.assignees = vec!["alice".to_string()];
        let mut issue2 = make_issue(2, "Assigned to bob", "open", vec![]);
        issue2.assignees = vec!["bob".to_string()];
        let issue3 = make_issue(3, "Unassigned", "open", vec![]);

        save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            Some("alice"),
            false,
            None,
            "priority",
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].number, 1);
    }

    #[test]
    fn test_filter_unassigned() {
        let conn = test_db();
        let mut issue1 = make_issue(1, "Assigned", "open", vec![]);
        issue1.assignees = vec!["alice".to_string()];
        let issue2 = make_issue(2, "Unassigned", "open", vec![]);

        save_issues(&conn, "owner/repo", &[issue1, issue2], true, true).unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            None,
            true,
            None,
            "priority",
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].number, 2);
    }

    #[test]
    fn test_filter_by_goal() {
        let conn = test_db();
        let mut issue1 = make_issue(1, "In v1.0", "open", vec![]);
        issue1.milestone = Some("v1.0".to_string());
        let mut issue2 = make_issue(2, "In v2.0", "open", vec![]);
        issue2.milestone = Some("v2.0".to_string());
        let issue3 = make_issue(3, "No milestone", "open", vec![]);

        save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            None,
            false,
            Some("v1.0"),
            "priority",
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].number, 1);
    }

    #[test]
    fn test_sort_by_priority() {
        let conn = test_db();
        let mut issue1 = make_issue(1, "Low priority", "open", vec![]);
        issue1.priority = 3;
        let mut issue2 = make_issue(2, "High priority", "open", vec![]);
        issue2.priority = 1;
        let mut issue3 = make_issue(3, "Urgent", "open", vec![]);
        issue3.priority = 0;

        save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            None,
            false,
            None,
            "priority",
        )
        .unwrap();
        assert_eq!(results.len(), 3);
        // Priority ASC: 0, 1, 3
        assert_eq!(results[0].priority, 0);
        assert_eq!(results[1].priority, 1);
        assert_eq!(results[2].priority, 3);
    }

    #[test]
    fn test_sort_by_newest() {
        let conn = test_db();
        save_issues(
            &conn,
            "owner/repo",
            &[
                make_issue(1, "Oldest", "open", vec![]),
                make_issue(2, "Middle", "open", vec![]),
                make_issue(3, "Newest", "open", vec![]),
            ],
            true,
            true,
        )
        .unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            None,
            false,
            None,
            "newest",
        )
        .unwrap();
        assert_eq!(results[0].number, 3);
        assert_eq!(results[1].number, 2);
        assert_eq!(results[2].number, 1);
    }

    #[test]
    fn test_sort_by_oldest() {
        let conn = test_db();
        save_issues(
            &conn,
            "owner/repo",
            &[
                make_issue(1, "Oldest", "open", vec![]),
                make_issue(2, "Middle", "open", vec![]),
                make_issue(3, "Newest", "open", vec![]),
            ],
            true,
            true,
        )
        .unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            None,
            false,
            None,
            "oldest",
        )
        .unwrap();
        assert_eq!(results[0].number, 1);
        assert_eq!(results[1].number, 2);
        assert_eq!(results[2].number, 3);
    }

    #[test]
    fn test_sort_by_updated() {
        let conn = test_db();
        let mut issue1 = make_issue(1, "Updated last", "open", vec![]);
        issue1.updated_at = "2024-01-03T00:00:00Z".to_string();
        let mut issue2 = make_issue(2, "Updated first", "open", vec![]);
        issue2.updated_at = "2024-01-01T00:00:00Z".to_string();
        let mut issue3 = make_issue(3, "Updated middle", "open", vec![]);
        issue3.updated_at = "2024-01-02T00:00:00Z".to_string();

        save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            None,
            None,
            None,
            false,
            None,
            "updated",
        )
        .unwrap();
        // DESC by updated_at
        assert_eq!(results[0].number, 1); // 2024-01-03
        assert_eq!(results[1].number, 3); // 2024-01-02
        assert_eq!(results[2].number, 2); // 2024-01-01
    }

    #[test]
    fn test_combined_filters() {
        let conn = test_db();
        let mut issue1 = make_issue(1, "Match all", "open", vec!["bug"]);
        issue1.assignees = vec!["alice".to_string()];
        issue1.milestone = Some("v1.0".to_string());

        let mut issue2 = make_issue(2, "Wrong state", "closed", vec!["bug"]);
        issue2.assignees = vec!["alice".to_string()];
        issue2.milestone = Some("v1.0".to_string());

        let mut issue3 = make_issue(3, "Wrong assignee", "open", vec!["bug"]);
        issue3.assignees = vec!["bob".to_string()];
        issue3.milestone = Some("v1.0".to_string());

        save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

        let results = load_issues_filtered(
            &conn,
            "owner/repo",
            None,
            Some("bug"),
            Some("open"),
            Some("alice"),
            false,
            Some("v1.0"),
            "priority",
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].number, 1);
    }
}
