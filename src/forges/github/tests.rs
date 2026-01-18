//! Unit tests for GitHub module

use super::priority::apply_priority_from_labels;
use crate::forges::{Issue, Label};

fn make_issue(id: &str, labels: Vec<&str>) -> Issue {
    Issue {
        id: id.to_string(),
        title: format!("Issue {}", id),
        body: None,
        state: "open".to_string(),
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
        parent_id: None,
    }
}

#[test]
fn test_apply_priority_from_labels() {
    let config: toml::Value = toml::from_str(
        r#"
            P0 = 0
            P1 = 1
            P2 = 2
        "#,
    )
    .unwrap();

    let mut issues = vec![make_issue("1", vec!["P0", "bug"])];
    apply_priority_from_labels(&mut issues, &config);

    assert_eq!(issues[0].priority, 0);
    assert_eq!(issues[0].priority_label, Some("P0".to_string()));
}

#[test]
fn test_priority_uses_lowest_value() {
    let config: toml::Value = toml::from_str(
        r#"
            P0 = 0
            P1 = 1
        "#,
    )
    .unwrap();

    // Issue has both P1 (priority 1) and P0 (priority 0) - should pick P0
    let mut issues = vec![make_issue("1", vec!["P1", "P0"])];
    apply_priority_from_labels(&mut issues, &config);

    assert_eq!(issues[0].priority, 0);
    assert_eq!(issues[0].priority_label, Some("P0".to_string()));
}

#[test]
fn test_priority_no_matching_labels() {
    let config: toml::Value = toml::from_str(
        r#"
            P0 = 0
            P1 = 1
        "#,
    )
    .unwrap();

    let mut issues = vec![make_issue("1", vec!["bug", "enhancement"])];
    apply_priority_from_labels(&mut issues, &config);

    // Should remain at default priority
    assert_eq!(issues[0].priority, 4);
    assert_eq!(issues[0].priority_label, None);
}

#[test]
fn test_priority_empty_config() {
    let config: toml::Value = toml::from_str("").unwrap();

    let mut issues = vec![make_issue("1", vec!["P0"])];
    apply_priority_from_labels(&mut issues, &config);

    // No config means no priority mapping
    assert_eq!(issues[0].priority, 4);
}

#[test]
fn test_priority_invalid_config_values() {
    let config: toml::Value = toml::from_str(
        r#"
            P0 = 0
            bad = 99
            negative = -1
        "#,
    )
    .unwrap();

    // P0 should work, but bad/negative should be ignored
    let mut issues = vec![make_issue("1", vec!["P0"]), make_issue("2", vec!["bad"])];
    apply_priority_from_labels(&mut issues, &config);

    assert_eq!(issues[0].priority, 0); // P0 works
    assert_eq!(issues[1].priority, 4); // bad value ignored
}
