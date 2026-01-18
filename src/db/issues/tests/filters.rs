//! Basic filtering tests

use super::{make_issue, test_db};
use crate::db::issues::{load_issues_filtered, save_issues};

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
