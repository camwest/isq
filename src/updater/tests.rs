//! Tests for updater module

use semver::Version;

use super::staged::{staged_update_dir, staged_update_path};
use super::version::parse_version_from_output;

#[test]
fn test_version_comparison() {
    // These tests verify semver comparison logic
    let v1 = Version::parse("0.1.0").unwrap();
    let v2 = Version::parse("0.2.0").unwrap();
    assert!(v2 > v1);

    let v3 = Version::parse("0.2.0-beta").unwrap();
    assert!(v2 > v3); // stable > prerelease

    let v4 = Version::parse("1.0.0").unwrap();
    let v5 = Version::parse("0.99.99").unwrap();
    assert!(v4 > v5);
}

#[test]
fn test_prerelease_detection() {
    assert!("0.2.0-beta".contains('-'));
    assert!("0.2.0-rc.1".contains('-'));
    assert!(!"0.2.0".contains('-'));
    assert!(!"1.0.0".contains('-'));
}

#[test]
fn test_parse_version_from_output() {
    // Basic format
    assert_eq!(parse_version_from_output("isq 0.1.0").unwrap(), "0.1.0");

    // With install method suffix
    assert_eq!(
        parse_version_from_output("isq 0.2.0 (standalone)").unwrap(),
        "0.2.0"
    );

    // With auto-update info
    assert_eq!(
        parse_version_from_output("isq 1.0.0 (standalone, auto-updates enabled)").unwrap(),
        "1.0.0"
    );

    // With trailing newline
    assert_eq!(parse_version_from_output("isq 0.1.0\n").unwrap(), "0.1.0");

    // Multi-line output (homebrew includes note)
    assert_eq!(
        parse_version_from_output("isq 0.1.0 (homebrew)\nNote: Run `brew upgrade isq` to update.")
            .unwrap(),
        "0.1.0"
    );
}

#[test]
fn test_parse_version_from_output_errors() {
    // Empty string
    assert!(parse_version_from_output("").is_err());

    // Wrong prefix
    assert!(parse_version_from_output("foo 0.1.0").is_err());

    // No version
    assert!(parse_version_from_output("isq").is_err());

    // Just whitespace
    assert!(parse_version_from_output("   ").is_err());
}

#[test]
fn test_staged_update_path_format() {
    // Verify the staged update path is in the expected location
    if let Ok(path) = staged_update_path() {
        assert!(path.to_string_lossy().contains("staged-update"));
        assert!(path.to_string_lossy().ends_with("isq"));
    }
}

#[test]
fn test_staged_update_dir_format() {
    // Verify the staged update dir is in the expected location
    if let Ok(path) = staged_update_dir() {
        assert!(path.to_string_lossy().contains("staged-update"));
    }
}
