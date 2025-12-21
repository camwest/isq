//! E2E performance tests for isq
//!
//! These tests hit real GitHub API - no mocks.
//! Run with: cargo test --test e2e
//!
//! Requirements:
//! - gh CLI authenticated
//! - Network access to GitHub

use std::process::Command;
use std::time::{Duration, Instant};

const TEST_REPO: &str = "anthropics/claude-code";
const MIN_EXPECTED_ISSUES: usize = 5000;

// Performance thresholds
// Sync is network-bound and varies with GitHub API conditions (5-60s)
// Cache reads are what matter for "instant" feel
const MAX_SYNC_TIME: Duration = Duration::from_secs(60);
const MAX_LIST_TIME: Duration = Duration::from_millis(100);
const MAX_SHOW_TIME: Duration = Duration::from_millis(50);

fn isq_binary() -> std::path::PathBuf {
    // Use release binary for accurate performance testing
    // Build with: cargo build --release
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    std::path::PathBuf::from(manifest_dir)
        .join("target")
        .join("release")
        .join("isq")
}

/// Sync 5k+ issues from anthropics/claude-code, then verify cache reads are instant.
#[test]
fn test_sync() {
    // First, we need to set up the repo context
    // Since isq uses git remote detection, we'll test via the sync command
    // by temporarily creating a git repo pointing to claude-code

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Initialize git repo with claude-code as origin
    Command::new("git")
        .args(["init"])
        .current_dir(temp_path)
        .output()
        .expect("git init failed");

    Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            &format!("https://github.com/{}.git", TEST_REPO),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git remote add failed");

    // Run sync
    let start = Instant::now();
    let output = Command::new(isq_binary())
        .args(["sync"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute isq sync");
    let sync_time = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Print output for debugging
    eprintln!("=== SYNC OUTPUT ===");
    eprintln!("stdout: {}", stdout);
    eprintln!("stderr: {}", stderr);
    eprintln!("time: {:?}", sync_time);
    eprintln!("===================");

    assert!(
        output.status.success(),
        "Sync failed: {}{}",
        stdout,
        stderr
    );

    assert!(
        sync_time < MAX_SYNC_TIME,
        "Sync too slow: {:?} (max {:?})",
        sync_time,
        MAX_SYNC_TIME
    );

    // Verify we synced a reasonable number of issues
    assert!(
        stdout.contains("Synced") || stderr.contains("Synced"),
        "Expected sync confirmation in output"
    );

    // Run list from cache - must be fast
    let start = Instant::now();
    let output = Command::new(isq_binary())
        .args(["issue", "list", "--json"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute isq issue list");
    let list_time = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);

    eprintln!("=== LIST OUTPUT ===");
    eprintln!("time: {:?}", list_time);
    eprintln!("===================");

    assert!(
        output.status.success(),
        "List failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        list_time < MAX_LIST_TIME,
        "List too slow: {:?} (max {:?}) - cache read should be instant",
        list_time,
        MAX_LIST_TIME
    );

    // Parse JSON and verify issue count
    let issues: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("Failed to parse JSON output");

    assert!(
        issues.len() >= MIN_EXPECTED_ISSUES,
        "Expected at least {} issues, got {}",
        MIN_EXPECTED_ISSUES,
        issues.len()
    );

    eprintln!(
        "SUCCESS: Synced {} issues in {:?}, listed in {:?}",
        issues.len(),
        sync_time,
        list_time
    );
}

/// Test that `isq issue show <id>` is fast from cache
#[test]
fn test_show_from_cache() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Initialize git repo with claude-code as origin
    Command::new("git")
        .args(["init"])
        .current_dir(temp_path)
        .output()
        .expect("git init failed");

    Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            &format!("https://github.com/{}.git", TEST_REPO),
        ])
        .current_dir(temp_path)
        .output()
        .expect("git remote add failed");

    // Sync first (needed to populate cache)
    let output = Command::new(isq_binary())
        .args(["sync"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute isq sync");

    assert!(output.status.success(), "Sync failed");

    // Get an issue number from list
    let output = Command::new(isq_binary())
        .args(["issue", "list", "--json"])
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute isq issue list");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let issues: Vec<serde_json::Value> = serde_json::from_str(&stdout).expect("Failed to parse");
    let issue_number = issues[0]["number"].as_u64().expect("No issue number");

    // Show that issue - must be fast
    let start = Instant::now();
    let output = Command::new(isq_binary())
        .args(["issue", "show", &issue_number.to_string()])
        .current_dir(temp_path)
        .output()
        .expect("Failed to execute isq issue show");
    let show_time = start.elapsed();

    eprintln!("=== SHOW OUTPUT ===");
    eprintln!("time: {:?}", show_time);
    eprintln!("===================");

    assert!(
        output.status.success(),
        "Show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        show_time < MAX_SHOW_TIME,
        "Show too slow: {:?} (max {:?})",
        show_time,
        MAX_SHOW_TIME
    );
}
