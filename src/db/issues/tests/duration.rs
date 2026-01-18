//! Duration parsing tests

use crate::db::issues::parse_duration_to_sqlite_modifier;

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
