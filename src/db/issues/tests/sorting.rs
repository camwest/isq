//! Sorting tests

use super::{make_issue, test_db};
use crate::db::issues::{load_issues_filtered, save_issues};

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
