//! Tests for display module.

use super::terminal::{luminance, parse_hex_color};
use super::time::relative_time;

#[test]
fn test_relative_time() {
    // Just test the function doesn't panic on various inputs
    assert_eq!(relative_time("invalid"), "invalid");
    assert!(!relative_time("2024-01-01T00:00:00Z").is_empty());
}

#[test]
fn test_parse_hex_color_valid() {
    assert_eq!(parse_hex_color("ff0000"), Some((255, 0, 0)));
    assert_eq!(parse_hex_color("00ff00"), Some((0, 255, 0)));
    assert_eq!(parse_hex_color("0000ff"), Some((0, 0, 255)));
    assert_eq!(parse_hex_color("4EA7FC"), Some((78, 167, 252)));
}

#[test]
fn test_parse_hex_color_with_hash() {
    assert_eq!(parse_hex_color("#ff0000"), Some((255, 0, 0)));
    assert_eq!(parse_hex_color("#4EA7FC"), Some((78, 167, 252)));
}

#[test]
fn test_parse_hex_color_invalid() {
    assert_eq!(parse_hex_color(""), None);
    assert_eq!(parse_hex_color("fff"), None); // Too short
    assert_eq!(parse_hex_color("fffffff"), None); // Too long
    assert_eq!(parse_hex_color("gggggg"), None); // Invalid hex chars
}

#[test]
fn test_luminance() {
    // Black
    assert_eq!(luminance(0, 0, 0), 0.0);
    // White
    assert!((luminance(255, 255, 255) - 255.0).abs() < 0.1);
    // Pure red (0.299 * 255 = 76.245)
    assert!((luminance(255, 0, 0) - 76.245).abs() < 0.1);
}

#[test]
fn test_priority_indicator() {
    use super::issues::priority_indicator;
    assert_eq!(priority_indicator(0), "[!]"); // Urgent
    assert_eq!(priority_indicator(1), "▰▰▰"); // High
    assert_eq!(priority_indicator(2), "▰▰▱"); // Medium
    assert_eq!(priority_indicator(3), "▰▱▱"); // Low
    assert_eq!(priority_indicator(4), "---"); // None
    assert_eq!(priority_indicator(5), "---"); // Unknown defaults to none
    assert_eq!(priority_indicator(255), "---"); // Edge case
}
