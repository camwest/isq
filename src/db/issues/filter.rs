//! Issue filtering and query building

use anyhow::Result;
use rusqlite::{Connection, Row};

use crate::forges::Issue;

use super::parse_labels_json;

/// Convert a database row to an Issue struct.
/// Expected column order: issue_id, title, body, state, author, labels, assignees,
/// priority, priority_label, created_at, updated_at, html_url, milestone
fn row_to_issue(row: &Row) -> rusqlite::Result<Issue> {
    let labels_json: String = row.get(5)?;
    let labels = parse_labels_json(&labels_json);
    let assignees_json: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
    let assignees: Vec<String> = serde_json::from_str(&assignees_json).unwrap_or_default();
    let priority: i64 = row.get::<_, Option<i64>>(7)?.unwrap_or(4);

    Ok(Issue {
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
    })
}

/// Filter parameters for loading issues
#[derive(Default)]
pub struct IssueFilter<'a> {
    pub ids: Option<&'a [&'a str]>,
    pub label: Option<&'a str>,
    pub label_not: Option<&'a str>,
    pub label_any: Option<&'a [String]>,
    pub state: Option<&'a str>,
    pub assignee: Option<&'a str>,
    pub unassigned: bool,
    pub goal: Option<&'a str>,
    pub priority: Option<u8>,
    pub priority_lte: Option<u8>,
    pub priority_gte: Option<u8>,
    pub updated_before: Option<&'a str>,
    pub updated_after: Option<&'a str>,
    pub created_before: Option<&'a str>,
    pub created_after: Option<&'a str>,
    pub sort: &'a str,
}

/// Load issues with optional filters
#[allow(clippy::too_many_arguments)]
pub fn load_issues_filtered(
    conn: &Connection,
    repo: &str,
    ids: Option<&[&str]>,
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
        "SELECT issue_id, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone
         FROM issues WHERE repo = ? AND deleted = 0",
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(repo.to_string())];

    // Filter by specific IDs if provided
    if let Some(id_list) = ids
        && !id_list.is_empty()
    {
        let placeholders: Vec<&str> = id_list.iter().map(|_| "?").collect();
        sql.push_str(&format!(" AND issue_id IN ({})", placeholders.join(",")));
        for id in id_list {
            params_vec.push(Box::new(id.to_string()));
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
        "newest" => "created_at DESC",
        "oldest" => "created_at ASC",
        "updated" => "updated_at DESC",
        _ => "priority ASC, created_at DESC", // default: priority
    };
    sql.push_str(" ORDER BY ");
    sql.push_str(order_by);

    let mut stmt = conn.prepare(&sql)?;

    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let issues = stmt
        .query_map(params_refs.as_slice(), row_to_issue)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(issues)
}

/// Load issues with filter struct (supports all view filter fields)
pub fn load_issues_with_filter(
    conn: &Connection,
    repo: &str,
    filter: &IssueFilter,
) -> Result<Vec<Issue>> {
    let mut sql = String::from(
        "SELECT issue_id, title, body, state, author, labels, assignees, priority, priority_label, created_at, updated_at, html_url, milestone
         FROM issues WHERE repo = ? AND deleted = 0",
    );

    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(repo.to_string())];

    // Filter by specific IDs
    if let Some(id_list) = filter.ids
        && !id_list.is_empty()
    {
        let placeholders: Vec<&str> = id_list.iter().map(|_| "?").collect();
        sql.push_str(&format!(" AND issue_id IN ({})", placeholders.join(",")));
        for id in id_list {
            params_vec.push(Box::new(id.to_string()));
        }
    }

    // Filter by state
    if let Some(s) = filter.state {
        sql.push_str(" AND state = ?");
        params_vec.push(Box::new(s.to_string()));
    }

    // Filter by label (include)
    if let Some(l) = filter.label {
        sql.push_str(" AND labels LIKE ?");
        params_vec.push(Box::new(format!("%\"{}\"%", l)));
    }

    // Filter by label (exclude)
    if let Some(l) = filter.label_not {
        sql.push_str(" AND labels NOT LIKE ?");
        params_vec.push(Box::new(format!("%\"{}\"%", l)));
    }

    // Filter by any of these labels (OR)
    if let Some(labels) = filter.label_any
        && !labels.is_empty()
    {
        let conditions: Vec<String> = labels.iter().map(|_| "labels LIKE ?".to_string()).collect();
        sql.push_str(&format!(" AND ({})", conditions.join(" OR ")));
        for label in labels {
            params_vec.push(Box::new(format!("%\"{}\"%", label)));
        }
    }

    // Filter by assignee
    if let Some(a) = filter.assignee {
        sql.push_str(" AND assignees LIKE ?");
        params_vec.push(Box::new(format!("%\"{}\"%", a)));
    }

    // Filter unassigned
    if filter.unassigned {
        sql.push_str(" AND (assignees = '[]' OR assignees IS NULL OR assignees = '')");
    }

    // Filter by goal/milestone
    if let Some(g) = filter.goal {
        sql.push_str(" AND milestone = ?");
        params_vec.push(Box::new(g.to_string()));
    }

    // Priority filters
    if let Some(p) = filter.priority {
        sql.push_str(" AND priority = ?");
        params_vec.push(Box::new(p as i64));
    }
    if let Some(p) = filter.priority_lte {
        sql.push_str(" AND priority <= ?");
        params_vec.push(Box::new(p as i64));
    }
    if let Some(p) = filter.priority_gte {
        sql.push_str(" AND priority >= ?");
        params_vec.push(Box::new(p as i64));
    }

    // Date filters - parse human-readable durations like "30 days", "2 weeks"
    if let Some(duration) = filter.updated_before
        && let Some(modifier) = parse_duration_to_sqlite_modifier(duration)
    {
        sql.push_str(&format!(
            " AND updated_at < datetime('now', '{}')",
            modifier
        ));
    }
    if let Some(duration) = filter.updated_after
        && let Some(modifier) = parse_duration_to_sqlite_modifier(duration)
    {
        sql.push_str(&format!(
            " AND updated_at >= datetime('now', '{}')",
            modifier
        ));
    }

    // Created date filters
    if let Some(duration) = filter.created_before
        && let Some(modifier) = parse_duration_to_sqlite_modifier(duration)
    {
        sql.push_str(&format!(
            " AND created_at < datetime('now', '{}')",
            modifier
        ));
    }
    if let Some(duration) = filter.created_after
        && let Some(modifier) = parse_duration_to_sqlite_modifier(duration)
    {
        sql.push_str(&format!(
            " AND created_at >= datetime('now', '{}')",
            modifier
        ));
    }

    // Sort order
    let order_by = match filter.sort {
        "newest" => "created_at DESC",
        "oldest" => "created_at ASC",
        "updated" => "updated_at DESC",
        _ => "priority ASC, created_at DESC",
    };
    sql.push_str(" ORDER BY ");
    sql.push_str(order_by);

    let mut stmt = conn.prepare(&sql)?;
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let issues = stmt
        .query_map(params_refs.as_slice(), row_to_issue)?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(issues)
}

/// Parse human-readable duration to SQLite date modifier
/// Examples: "30 days" -> "-30 days", "2 weeks" -> "-14 days"
pub(crate) fn parse_duration_to_sqlite_modifier(duration: &str) -> Option<String> {
    let parts: Vec<&str> = duration.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }

    let num: i64 = parts[0].parse().ok()?;
    let unit = parts[1].to_lowercase();

    let days = match unit.as_str() {
        "day" | "days" => num,
        "week" | "weeks" => num * 7,
        "month" | "months" => num * 30,
        "year" | "years" => num * 365,
        _ => return None,
    };

    Some(format!("-{} days", days))
}
