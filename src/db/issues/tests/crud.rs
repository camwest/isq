//! CRUD operation tests

use super::{make_issue, test_db};
use crate::db::issues::{load_issue, load_issues, save_issues};

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
