//! Tests for daemon functionality.

use super::*;
use futures::stream::{self, StreamExt};
use queue::{ConflictKind, classify_error};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[test]
fn test_calculate_backoff_base_case() {
    // 0 failures = base interval (15s) with jitter
    let backoff = calculate_backoff(0);
    let secs = backoff.as_secs_f64();

    // Base is 15s, jitter is ±25%, so range is 11.25 to 18.75
    assert!(secs >= 11.25, "backoff {} too low for 0 failures", secs);
    assert!(secs <= 18.75, "backoff {} too high for 0 failures", secs);
}

#[test]
fn test_calculate_backoff_exponential_growth() {
    // Test that backoff grows exponentially (within jitter bounds)
    // 1 failure = 30s base, 2 = 60s, 3 = 120s, etc.

    let b1 = calculate_backoff(1);
    let b2 = calculate_backoff(2);
    let b3 = calculate_backoff(3);

    // With ±25% jitter: 1 failure = 22.5-37.5s, 2 = 45-75s, 3 = 90-150s
    assert!(
        b1.as_secs_f64() >= 22.5 && b1.as_secs_f64() <= 37.5,
        "1 failure backoff {} out of range",
        b1.as_secs_f64()
    );
    assert!(
        b2.as_secs_f64() >= 45.0 && b2.as_secs_f64() <= 75.0,
        "2 failure backoff {} out of range",
        b2.as_secs_f64()
    );
    assert!(
        b3.as_secs_f64() >= 90.0 && b3.as_secs_f64() <= 150.0,
        "3 failure backoff {} out of range",
        b3.as_secs_f64()
    );
}

#[test]
fn test_calculate_backoff_caps_at_max() {
    // Exponent caps at 6: 15 * 2^6 = 960s max
    // With ±25% jitter: 720 to 1200
    let backoff = calculate_backoff(10);
    let secs = backoff.as_secs_f64();

    assert!(secs >= 720.0, "max backoff {} too low", secs);
    assert!(secs <= 1200.0, "max backoff {} too high", secs);
}

#[test]
fn test_calculate_backoff_very_high_failures() {
    // Even with extreme failures, should not overflow and should cap at 960s
    let backoff = calculate_backoff(100);
    let secs = backoff.as_secs_f64();

    // Should be capped at 960s with ±25% jitter = 720 to 1200
    assert!(
        (720.0..=1200.0).contains(&secs),
        "extreme failure backoff {} should be capped",
        secs
    );
}

#[test]
fn test_calculate_backoff_has_jitter() {
    // Run multiple times and verify we get different values (jitter working)
    let mut values: Vec<f64> = Vec::new();
    for _ in 0..10 {
        values.push(calculate_backoff(2).as_secs_f64());
    }

    // Check that not all values are identical (jitter is applied)
    let first = values[0];
    let has_variation = values.iter().any(|&v| (v - first).abs() > 0.001);
    assert!(has_variation, "backoff should have jitter variation");
}

#[tokio::test]
async fn test_parallel_sync_executes_concurrently() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let counter = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let c = Arc::clone(&counter);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<_, anyhow::Error>(())
            }
        })
        .collect();

    let start = Instant::now();
    let results: Vec<_> = stream::iter(tasks).buffer_unordered(4).collect().await;
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 4);
    assert_eq!(counter.load(Ordering::SeqCst), 4);
    // Parallel: ~100ms. Sequential would be ~400ms.
    assert!(
        elapsed < Duration::from_millis(250),
        "took {:?}, expected parallel execution",
        elapsed
    );
}

#[tokio::test]
async fn test_backoff_state_updates_from_parallel_results() {
    let states: Arc<Mutex<HashMap<String, RepoSyncState>>> = Arc::new(Mutex::new(HashMap::new()));

    // Simulate parallel sync results
    let results = vec![
        ("repo1".to_string(), SyncResult::Success),
        (
            "repo2".to_string(),
            SyncResult::Error(anyhow::anyhow!("network error")),
        ),
        ("repo3".to_string(), SyncResult::Skipped),
    ];

    let now = Instant::now();
    {
        let mut s = states.lock().await;
        for (repo, result) in results {
            match result {
                SyncResult::Success => {
                    s.remove(&repo);
                }
                SyncResult::Skipped => {}
                SyncResult::Error(_) => {
                    let state = s.entry(repo).or_insert(RepoSyncState {
                        consecutive_failures: 0,
                        next_attempt: now,
                    });
                    state.consecutive_failures += 1;
                    state.next_attempt = now + calculate_backoff(state.consecutive_failures);
                }
            }
        }
    }

    let s = states.lock().await;
    assert!(!s.contains_key("repo1"), "success should remove backoff");
    assert!(s.contains_key("repo2"), "error should add backoff");
    assert_eq!(s.get("repo2").unwrap().consecutive_failures, 1);
    assert!(!s.contains_key("repo3"), "skipped should not modify state");
}

// =============================================================================
// Conflict Resolution Tests
// =============================================================================

/// Test: GitHub 404 error is classified as NotFound
#[test]
fn test_classify_error_github_404() {
    let err = anyhow::anyhow!("GitHub API error 404 Not Found: {{\"message\":\"Not Found\"}}");
    assert_eq!(classify_error(&err), Some(ConflictKind::NotFound));
}

/// Test: GitHub 422 error is classified as ValidationError
#[test]
fn test_classify_error_github_422() {
    let err = anyhow::anyhow!(
        "GitHub API error 422 Unprocessable Entity: {{\"message\":\"Validation Failed\"}}"
    );
    assert_eq!(classify_error(&err), Some(ConflictKind::ValidationError));
}

/// Test: GitHub 409 error is classified as StateConflict
#[test]
fn test_classify_error_github_409() {
    let err = anyhow::anyhow!("GitHub API error 409 Conflict: {{\"message\":\"Conflict\"}}");
    assert_eq!(classify_error(&err), Some(ConflictKind::StateConflict));
}

/// Test: JIRA 404 error (parenthesized format) is classified as NotFound
#[test]
fn test_classify_error_jira_404() {
    let err = anyhow::anyhow!("JIRA API error (404): Issue does not exist");
    assert_eq!(classify_error(&err), Some(ConflictKind::NotFound));
}

/// Test: JIRA transition error is classified as StateConflict
#[test]
fn test_classify_error_jira_no_transition() {
    let err = anyhow::anyhow!("No 'Done' transition available for this issue");
    assert_eq!(classify_error(&err), Some(ConflictKind::StateConflict));
}

/// Test: Linear GraphQL "not found" error is classified as NotFound
#[test]
fn test_classify_error_linear_not_found() {
    let err = anyhow::anyhow!("Linear GraphQL errors: Entity not found");
    assert_eq!(classify_error(&err), Some(ConflictKind::NotFound));
}

/// Test: Linear issue not found error is classified as NotFound
#[test]
fn test_classify_error_linear_issue_not_found() {
    let err = anyhow::anyhow!("Issue #123 not found in team");
    assert_eq!(classify_error(&err), Some(ConflictKind::NotFound));
}

/// Test: "already closed" semantic pattern is classified as StateConflict
#[test]
fn test_classify_error_already_closed() {
    let err = anyhow::anyhow!("Cannot comment: issue is already closed");
    assert_eq!(classify_error(&err), Some(ConflictKind::StateConflict));
}

/// Test: "already exists" semantic pattern is classified as StateConflict
#[test]
fn test_classify_error_already_exists() {
    let err = anyhow::anyhow!("Label already exists on this issue");
    assert_eq!(classify_error(&err), Some(ConflictKind::StateConflict));
}

/// Test: Network errors are NOT classified as conflicts (should retry)
#[test]
fn test_classify_error_network_not_conflict() {
    let cases = [
        "connection refused",
        "network is unreachable",
        "DNS resolution failed",
        "connection timed out",
        "connection reset by peer",
    ];

    for msg in cases {
        let err = anyhow::anyhow!("{}", msg);
        assert_eq!(
            classify_error(&err),
            None,
            "Network error '{}' should not be a conflict",
            msg
        );
    }
}

/// Test: Rate limit errors are NOT classified as conflicts (should retry)
#[test]
fn test_classify_error_rate_limit_not_conflict() {
    // Rate limits use 429 or 403, but we don't want to treat them as conflicts
    // since they're transient. The current implementation doesn't special-case
    // rate limits in classify_error (they're handled separately in sync.rs).
    let err = anyhow::anyhow!("Rate limit exceeded, retry after 60 seconds");
    assert_eq!(classify_error(&err), None);
}

/// Test: Server-wins conflict scenario
/// User tried to comment on an issue that was closed remotely while offline
#[test]
fn test_scenario_server_wins_comment_on_closed_issue() {
    // Simulate: user queued a comment, but issue was closed on server
    // GitHub would return 422 "Issues are disabled" or similar
    let github_err = anyhow::anyhow!("GitHub API error 422 Unprocessable Entity: Issue was closed");
    assert!(classify_error(&github_err).is_some());

    // Linear would return a GraphQL error
    let linear_err = anyhow::anyhow!("Linear GraphQL errors: Issue is already closed");
    assert_eq!(
        classify_error(&linear_err),
        Some(ConflictKind::StateConflict)
    );

    // JIRA would return transition error
    let jira_err = anyhow::anyhow!("No 'Done' transition available for this issue");
    assert_eq!(classify_error(&jira_err), Some(ConflictKind::StateConflict));
}

/// Test: Partial success scenario
/// Label was deleted from repo during offline period
#[test]
fn test_scenario_partial_success_deleted_label() {
    // GitHub returns 404 when label doesn't exist
    let github_err = anyhow::anyhow!("GitHub API error 404 Not Found: Label not found");
    assert_eq!(classify_error(&github_err), Some(ConflictKind::NotFound));

    // Linear returns "not found" in GraphQL
    let linear_err = anyhow::anyhow!("Label 'urgent' not found");
    assert_eq!(classify_error(&linear_err), Some(ConflictKind::NotFound));
}

/// Test: State divergence scenario
/// Maintainer reopened issue while user tried closing it offline
#[test]
fn test_scenario_state_divergence_reopen_close_race() {
    // This typically results in success (close succeeds) or 409 conflict
    let conflict_err = anyhow::anyhow!("GitHub API error 409 Conflict: State has changed");
    assert_eq!(
        classify_error(&conflict_err),
        Some(ConflictKind::StateConflict)
    );

    // Or a semantic "already" message
    let semantic_err = anyhow::anyhow!("Issue state was modified by another user");
    assert_eq!(
        classify_error(&semantic_err),
        Some(ConflictKind::StateConflict)
    );
}

/// Test: Linear mutation failures are classified as StateConflict
/// These occur when GraphQL returns success: false
#[test]
fn test_classify_error_linear_mutation_failures() {
    let cases = [
        "Failed to create comment",
        "Failed to close issue",
        "Failed to reopen issue",
        "Failed to update issue",
        "Failed to add label",
        "Failed to remove label",
        "Failed to assign issue",
        "Failed to transition issue",
    ];

    for msg in cases {
        let err = anyhow::anyhow!("{}", msg);
        assert_eq!(
            classify_error(&err),
            Some(ConflictKind::StateConflict),
            "Linear mutation failure '{}' should be StateConflict",
            msg
        );
    }
}

/// Test: Linear workflow state not found is classified as ValidationError
#[test]
fn test_classify_error_linear_no_workflow_state() {
    let err = anyhow::anyhow!("No workflow state of type 'completed' found");
    assert_eq!(classify_error(&err), Some(ConflictKind::ValidationError));

    let err2 = anyhow::anyhow!("No workflow state matching 'Done' found");
    assert_eq!(classify_error(&err2), Some(ConflictKind::ValidationError));
}
