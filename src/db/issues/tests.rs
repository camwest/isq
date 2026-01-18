//! Tests for issue CRUD operations

use crate::db::schema::init_schema;
use crate::forges::{Issue, Label};
use rusqlite::Connection;

use super::{
    IssueFilter, load_issue, load_issues, load_issues_filtered, load_issues_with_filter,
    parse_labels_json, save_issues,
};

/// Create an in-memory database for testing
fn test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn
}

fn make_issue(id: &str, title: &str, state: &str, labels: Vec<&str>) -> Issue {
    Issue {
        id: id.to_string(),
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
        make_issue("1", "First", "open", vec![]),
        make_issue("2", "Second", "open", vec!["bug"]),
    ];

    save_issues(&conn, "owner/repo", &issues, true, true).unwrap();

    let loaded = load_issues(&conn, "owner/repo").unwrap();
    assert_eq!(loaded.len(), 2);
}

#[test]
fn test_save_issues_replaces_existing() {
    let conn = test_db();

    save_issues(
        &conn,
        "owner/repo",
        &[make_issue("1", "Old", "open", vec![])],
        true,
        true,
    )
    .unwrap();
    save_issues(
        &conn,
        "owner/repo",
        &[make_issue("2", "New", "open", vec![])],
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
        make_issue("1", "Open issue", "open", vec![]),
        make_issue("2", "Closed issue", "closed", vec![]),
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
        make_issue("1", "Bug", "open", vec!["bug"]),
        make_issue("2", "Feature", "open", vec!["enhancement"]),
        make_issue("3", "Bug and feature", "open", vec!["bug", "enhancement"]),
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
        &[make_issue("42", "The answer", "open", vec![])],
        true,
        true,
    )
    .unwrap();

    let issue = load_issue(&conn, "owner/repo", "42").unwrap();
    assert!(issue.is_some());
    assert_eq!(issue.unwrap().title, "The answer");

    let missing = load_issue(&conn, "owner/repo", "999").unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_filter_by_assignee() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "Assigned to alice", "open", vec![]);
    issue1.assignees = vec!["alice".to_string()];
    let mut issue2 = make_issue("2", "Assigned to bob", "open", vec![]);
    issue2.assignees = vec!["bob".to_string()];
    let issue3 = make_issue("3", "Unassigned", "open", vec![]);

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
    assert_eq!(results[0].id, "1");
}

#[test]
fn test_filter_unassigned() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "Assigned", "open", vec![]);
    issue1.assignees = vec!["alice".to_string()];
    let issue2 = make_issue("2", "Unassigned", "open", vec![]);

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
    assert_eq!(results[0].id, "2");
}

#[test]
fn test_filter_by_goal() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "In v1.0", "open", vec![]);
    issue1.milestone = Some("v1.0".to_string());
    let mut issue2 = make_issue("2", "In v2.0", "open", vec![]);
    issue2.milestone = Some("v2.0".to_string());
    let issue3 = make_issue("3", "No milestone", "open", vec![]);

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
    assert_eq!(results[0].id, "1");
}

#[test]
fn test_sort_by_priority() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "Low priority", "open", vec![]);
    issue1.priority = 3;
    let mut issue2 = make_issue("2", "High priority", "open", vec![]);
    issue2.priority = 1;
    let mut issue3 = make_issue("3", "Urgent", "open", vec![]);
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
    let mut i1 = make_issue("1", "Oldest", "open", vec![]);
    i1.created_at = "2024-01-01T00:00:00Z".to_string();
    let mut i2 = make_issue("2", "Middle", "open", vec![]);
    i2.created_at = "2024-01-02T00:00:00Z".to_string();
    let mut i3 = make_issue("3", "Newest", "open", vec![]);
    i3.created_at = "2024-01-03T00:00:00Z".to_string();
    save_issues(&conn, "owner/repo", &[i1, i2, i3], true, true).unwrap();

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
    assert_eq!(results[0].id, "3");
    assert_eq!(results[1].id, "2");
    assert_eq!(results[2].id, "1");
}

#[test]
fn test_sort_by_oldest() {
    let conn = test_db();
    let mut i1 = make_issue("1", "Oldest", "open", vec![]);
    i1.created_at = "2024-01-01T00:00:00Z".to_string();
    let mut i2 = make_issue("2", "Middle", "open", vec![]);
    i2.created_at = "2024-01-02T00:00:00Z".to_string();
    let mut i3 = make_issue("3", "Newest", "open", vec![]);
    i3.created_at = "2024-01-03T00:00:00Z".to_string();
    save_issues(&conn, "owner/repo", &[i1, i2, i3], true, true).unwrap();

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
    assert_eq!(results[0].id, "1");
    assert_eq!(results[1].id, "2");
    assert_eq!(results[2].id, "3");
}

#[test]
fn test_sort_by_updated() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "Updated last", "open", vec![]);
    issue1.updated_at = "2024-01-03T00:00:00Z".to_string();
    let mut issue2 = make_issue("2", "Updated first", "open", vec![]);
    issue2.updated_at = "2024-01-01T00:00:00Z".to_string();
    let mut issue3 = make_issue("3", "Updated middle", "open", vec![]);
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
    assert_eq!(results[0].id, "1"); // 2024-01-03
    assert_eq!(results[1].id, "3"); // 2024-01-02
    assert_eq!(results[2].id, "2"); // 2024-01-01
}

#[test]
fn test_combined_filters() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "Match all", "open", vec!["bug"]);
    issue1.assignees = vec!["alice".to_string()];
    issue1.milestone = Some("v1.0".to_string());

    let mut issue2 = make_issue("2", "Wrong state", "closed", vec!["bug"]);
    issue2.assignees = vec!["alice".to_string()];
    issue2.milestone = Some("v1.0".to_string());

    let mut issue3 = make_issue("3", "Wrong assignee", "open", vec!["bug"]);
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
    assert_eq!(results[0].id, "1");
}

// === Duration Parsing Tests ===

use super::parse_duration_to_sqlite_modifier;

#[test]
fn test_parse_duration_days() {
    assert_eq!(
        parse_duration_to_sqlite_modifier("30 days"),
        Some("-30 days".to_string())
    );
    assert_eq!(
        parse_duration_to_sqlite_modifier("1 day"),
        Some("-1 days".to_string())
    );
}

#[test]
fn test_parse_duration_weeks() {
    assert_eq!(
        parse_duration_to_sqlite_modifier("2 weeks"),
        Some("-14 days".to_string())
    );
    assert_eq!(
        parse_duration_to_sqlite_modifier("1 week"),
        Some("-7 days".to_string())
    );
}

#[test]
fn test_parse_duration_months() {
    assert_eq!(
        parse_duration_to_sqlite_modifier("1 month"),
        Some("-30 days".to_string())
    );
    assert_eq!(
        parse_duration_to_sqlite_modifier("3 months"),
        Some("-90 days".to_string())
    );
}

#[test]
fn test_parse_duration_invalid() {
    assert_eq!(parse_duration_to_sqlite_modifier("invalid"), None);
    assert_eq!(parse_duration_to_sqlite_modifier("30"), None);
    assert_eq!(parse_duration_to_sqlite_modifier("days"), None);
    assert_eq!(parse_duration_to_sqlite_modifier("30 hours"), None);
}

// === IssueFilter Tests ===

#[test]
fn test_filter_priority_exact() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "High", "open", vec![]);
    issue1.priority = 1;
    let mut issue2 = make_issue("2", "Medium", "open", vec![]);
    issue2.priority = 2;
    let mut issue3 = make_issue("3", "Low", "open", vec![]);
    issue3.priority = 3;

    save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

    let filter = IssueFilter {
        priority: Some(2),
        sort: "priority",
        ..Default::default()
    };
    let results = load_issues_with_filter(&conn, "owner/repo", &filter).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "2");
}

#[test]
fn test_filter_priority_lte() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "High", "open", vec![]);
    issue1.priority = 1;
    let mut issue2 = make_issue("2", "Medium", "open", vec![]);
    issue2.priority = 2;
    let mut issue3 = make_issue("3", "Low", "open", vec![]);
    issue3.priority = 3;

    save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

    let filter = IssueFilter {
        priority_lte: Some(2),
        sort: "priority",
        ..Default::default()
    };
    let results = load_issues_with_filter(&conn, "owner/repo", &filter).unwrap();
    assert_eq!(results.len(), 2);
    // priority ASC: 1, 2
    assert_eq!(results[0].priority, 1);
    assert_eq!(results[1].priority, 2);
}

#[test]
fn test_filter_priority_gte() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "High", "open", vec![]);
    issue1.priority = 1;
    let mut issue2 = make_issue("2", "Medium", "open", vec![]);
    issue2.priority = 2;
    let mut issue3 = make_issue("3", "Low", "open", vec![]);
    issue3.priority = 3;

    save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

    let filter = IssueFilter {
        priority_gte: Some(2),
        sort: "priority",
        ..Default::default()
    };
    let results = load_issues_with_filter(&conn, "owner/repo", &filter).unwrap();
    assert_eq!(results.len(), 2);
    // priority ASC: 2, 3
    assert_eq!(results[0].priority, 2);
    assert_eq!(results[1].priority, 3);
}

#[test]
fn test_filter_label_not() {
    let conn = test_db();
    let issues = vec![
        make_issue("1", "Bug", "open", vec!["bug"]),
        make_issue("2", "Feature", "open", vec!["enhancement"]),
        make_issue("3", "Wontfix bug", "open", vec!["bug", "wontfix"]),
    ];
    save_issues(&conn, "owner/repo", &issues, true, true).unwrap();

    // Get bugs but exclude wontfix
    let filter = IssueFilter {
        label: Some("bug"),
        label_not: Some("wontfix"),
        sort: "priority",
        ..Default::default()
    };
    let results = load_issues_with_filter(&conn, "owner/repo", &filter).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "1");
}

#[test]
fn test_filter_label_any() {
    let conn = test_db();
    let issues = vec![
        make_issue("1", "Bug", "open", vec!["bug"]),
        make_issue("2", "Feature", "open", vec!["enhancement"]),
        make_issue("3", "Security", "open", vec!["security"]),
        make_issue("4", "Docs", "open", vec!["documentation"]),
    ];
    save_issues(&conn, "owner/repo", &issues, true, true).unwrap();

    // Get issues with bug OR security label
    let labels = vec!["bug".to_string(), "security".to_string()];
    let filter = IssueFilter {
        label_any: Some(&labels),
        sort: "priority",
        ..Default::default()
    };
    let results = load_issues_with_filter(&conn, "owner/repo", &filter).unwrap();
    assert_eq!(results.len(), 2);
    // Should have issues 1 and 3 (bug and security)
    let ids: Vec<&str> = results.iter().map(|i| i.id.as_str()).collect();
    assert!(ids.contains(&"1"));
    assert!(ids.contains(&"3"));
}

#[test]
fn test_filter_struct_combined() {
    let conn = test_db();
    let mut issue1 = make_issue("1", "Good match", "open", vec!["bug"]);
    issue1.priority = 1;
    issue1.milestone = Some("v1.0".to_string());

    let mut issue2 = make_issue("2", "Wrong priority", "open", vec!["bug"]);
    issue2.priority = 4;
    issue2.milestone = Some("v1.0".to_string());

    let mut issue3 = make_issue("3", "Wontfix", "open", vec!["bug", "wontfix"]);
    issue3.priority = 1;
    issue3.milestone = Some("v1.0".to_string());

    save_issues(&conn, "owner/repo", &[issue1, issue2, issue3], true, true).unwrap();

    let filter = IssueFilter {
        label: Some("bug"),
        label_not: Some("wontfix"),
        priority_lte: Some(2),
        goal: Some("v1.0"),
        sort: "priority",
        ..Default::default()
    };
    let results = load_issues_with_filter(&conn, "owner/repo", &filter).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "1");
}
