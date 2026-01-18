//! System-level diagnostic checks (database, network)

use std::time::{Duration, Instant};

use anyhow::Result;

use super::types::{CheckDetails, CheckResult, DiagnosticCheck};
use crate::db;

/// Check database accessibility and integrity
pub fn check_database(verbose: bool) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // Check if we can get the database path
    let db_path = match db::db_path() {
        Ok(p) => p,
        Err(e) => {
            checks.push(DiagnosticCheck {
                category: "Database".to_string(),
                name: "Path".to_string(),
                result: CheckResult::Fail {
                    reason: format!("cannot determine path: {}", e),
                    guidance: "Check HOME or XDG_CACHE_HOME environment variables".to_string(),
                },
                details: None,
            });
            return checks;
        }
    };

    // Check if database exists and is accessible
    match db::open() {
        Ok(conn) => {
            // Get file size
            let size_str = std::fs::metadata(&db_path)
                .map(|m| format_size(m.len()))
                .unwrap_or_else(|_| "unknown".to_string());

            checks.push(DiagnosticCheck {
                category: "Database".to_string(),
                name: "Cache".to_string(),
                result: CheckResult::Pass,
                details: Some(CheckDetails {
                    source: None,
                    path: if verbose {
                        Some(db_path.display().to_string())
                    } else {
                        None
                    },
                    value: Some(format!("accessible ({})", size_str)),
                    latency_ms: None,
                }),
            });

            // Run integrity check
            check_database_integrity(&conn, &mut checks, &db_path);
        }
        Err(e) => {
            checks.push(DiagnosticCheck {
                category: "Database".to_string(),
                name: "Cache".to_string(),
                result: CheckResult::Fail {
                    reason: format!("cannot open: {}", e),
                    guidance: "Check file permissions and disk space".to_string(),
                },
                details: None,
            });
        }
    }

    checks
}

/// Run database integrity check
fn check_database_integrity(
    conn: &rusqlite::Connection,
    checks: &mut Vec<DiagnosticCheck>,
    db_path: &std::path::Path,
) {
    match conn.query_row::<String, _, _>("PRAGMA integrity_check", [], |row| row.get(0)) {
        Ok(result) if result == "ok" => {
            checks.push(DiagnosticCheck {
                category: "Database".to_string(),
                name: "Integrity".to_string(),
                result: CheckResult::Pass,
                details: Some(CheckDetails {
                    source: None,
                    path: None,
                    value: Some("OK".to_string()),
                    latency_ms: None,
                }),
            });
        }
        Ok(result) => {
            checks.push(DiagnosticCheck {
                category: "Database".to_string(),
                name: "Integrity".to_string(),
                result: CheckResult::Fail {
                    reason: format!("corruption detected: {}", result),
                    guidance: format!("Delete and resync: rm {} && isq sync", db_path.display()),
                },
                details: None,
            });
        }
        Err(e) => {
            checks.push(DiagnosticCheck {
                category: "Database".to_string(),
                name: "Integrity".to_string(),
                result: CheckResult::Fail {
                    reason: format!("check failed: {}", e),
                    guidance: format!("Delete and resync: rm {} && isq sync", db_path.display()),
                },
                details: None,
            });
        }
    }
}

/// Check network connectivity to forge APIs
pub fn check_network(verbose: bool) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    // Only check GitHub for now - it's the most common and has a simple endpoint
    let start = Instant::now();
    let result = check_github_api();
    let latency = start.elapsed();

    match result {
        Ok(()) => {
            checks.push(DiagnosticCheck {
                category: "Network".to_string(),
                name: "GitHub API".to_string(),
                result: CheckResult::Pass,
                details: Some(CheckDetails {
                    source: None,
                    path: None,
                    value: Some("reachable".to_string()),
                    latency_ms: if verbose {
                        Some(latency.as_millis() as u64)
                    } else {
                        None
                    },
                }),
            });
        }
        Err(e) => {
            checks.push(DiagnosticCheck {
                category: "Network".to_string(),
                name: "GitHub API".to_string(),
                result: CheckResult::Warn {
                    reason: format!("unreachable: {}", e),
                    guidance: "Check your network connection".to_string(),
                },
                details: None,
            });
        }
    }

    checks
}

/// Simple GitHub API connectivity check (no auth required)
fn check_github_api() -> Result<()> {
    // Use a simple HEAD request to the API root
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let resp = client
        .head("https://api.github.com")
        .header("User-Agent", "isq")
        .send()?;

    if resp.status().is_success() || resp.status().as_u16() == 403 {
        // 403 is fine - means we reached the API but need auth
        Ok(())
    } else {
        anyhow::bail!("HTTP {}", resp.status())
    }
}

/// Format file size in human-readable form
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }
}
