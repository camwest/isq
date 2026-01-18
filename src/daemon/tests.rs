//! Tests for daemon functionality.

use super::*;
use futures::stream::{self, StreamExt};
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
