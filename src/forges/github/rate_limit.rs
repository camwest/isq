//! GitHub rate limiting utilities

use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use tokio::sync::{Mutex, Semaphore};

// GitHub secondary rate limits (from docs):
// - Max 100 concurrent requests
// - Max 900 points/min (GET=1pt, POST/PATCH/PUT/DELETE=5pts)
// - Wait at least 1 sec between write requests
pub const MAX_CONCURRENT_REQUESTS: usize = 80; // Stay safely under 100
pub const WRITE_SPACING: Duration = Duration::from_secs(1);
pub const MAX_RETRIES: u32 = 3;
pub const PER_PAGE: usize = 100;

// Global rate limiting state
pub static REQUEST_SEMAPHORE: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)));
static LAST_WRITE_TIME: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

/// Throttle write requests to maintain 1 sec spacing
pub async fn throttle_write() {
    let mut last = LAST_WRITE_TIME.lock().await;
    if let Some(last_time) = *last {
        let elapsed = last_time.elapsed();
        if elapsed < WRITE_SPACING {
            tokio::time::sleep(WRITE_SPACING - elapsed).await;
        }
    }
    *last = Some(Instant::now());
}

/// Check if response indicates rate limiting
pub fn is_rate_limited(status: u16, body: &str) -> bool {
    (status == 403 || status == 429)
        && (body.contains("rate limit") || body.contains("secondary rate limit"))
}

/// Parse the last page number from GitHub's Link header
/// Example: <https://api.github.com/...?page=5>; rel="last"
pub fn parse_last_page_from_link_header(link_header: &str) -> Option<usize> {
    for part in link_header.split(',') {
        // Extract URL between < and >
        if part.contains("rel=\"last\"")
            && let Some(start) = part.find('<')
            && let Some(end) = part.find('>')
        {
            let url = &part[start + 1..end];
            // Extract page parameter (must be preceded by ? or &, not part of per_page)
            for prefix in ["?page=", "&page="] {
                if let Some(page_start) = url.find(prefix) {
                    let page_str = &url[page_start + prefix.len()..];
                    // Take digits until non-digit
                    let page_num: String = page_str
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    return page_num.parse().ok();
                }
            }
        }
    }
    None
}

/// Parse retry-after header or use exponential backoff
pub fn get_retry_delay(response: &reqwest::Response, attempt: u32) -> Duration {
    // Check retry-after header first
    if let Some(retry_after) = response.headers().get("retry-after")
        && let Ok(secs) = retry_after.to_str().unwrap_or("").parse::<u64>()
    {
        return Duration::from_secs(secs);
    }
    // Exponential backoff: 1s, 2s, 4s
    Duration::from_secs(1 << attempt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_link_header_with_last() {
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?per_page=100&page=2>; rel="next", <https://api.github.com/repos/foo/bar/issues/comments?per_page=100&page=5>; rel="last""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(5));
    }

    #[test]
    fn test_parse_link_header_last_first() {
        // GitHub sometimes returns rel="last" before rel="next"
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?page=10>; rel="last", <https://api.github.com/repos/foo/bar/issues/comments?page=2>; rel="next""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(10));
    }

    #[test]
    fn test_parse_link_header_no_last() {
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?page=2>; rel="next""#;
        assert_eq!(parse_last_page_from_link_header(header), None);
    }

    #[test]
    fn test_parse_link_header_empty() {
        assert_eq!(parse_last_page_from_link_header(""), None);
    }

    #[test]
    fn test_parse_link_header_large_page_number() {
        let header =
            r#"<https://api.github.com/repos/foo/bar/issues/comments?page=9999>; rel="last""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(9999));
    }

    #[test]
    fn test_parse_link_header_page_with_other_params() {
        // page= appears after other query params
        let header = r#"<https://api.github.com/repos/foo/bar/issues/comments?per_page=100&since=2024-01-01&page=42>; rel="last""#;
        assert_eq!(parse_last_page_from_link_header(header), Some(42));
    }
}
