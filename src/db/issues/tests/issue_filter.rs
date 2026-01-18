//! IssueFilter struct tests

use super::{make_issue, test_db};
use crate::db::issues::{IssueFilter, load_issues_with_filter, save_issues};

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
