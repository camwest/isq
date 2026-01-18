//! Label parsing tests

use crate::db::issues::parse_labels_json;

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
