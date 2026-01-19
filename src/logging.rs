//! Configurable logging with secret redaction.
//!
//! Provides structured logging via `tracing` with automatic redaction of
//! sensitive data like tokens and authorization headers.
//!
//! # Verbosity Levels
//!
//! CLI flags map to tracing levels:
//! - (default): WARN - errors and warnings only
//! - `-v`: INFO - user-facing progress
//! - `-vv`: DEBUG - detailed diagnostics
//! - `-vvv`: TRACE - full request/response debugging
//!
//! # Environment Variable Override
//!
//! `ISQ_LOG` overrides CLI verbosity with standard tracing filter syntax:
//! ```bash
//! ISQ_LOG=debug isq sync
//! ISQ_LOG=isq::forges::github=trace isq sync
//! ```
//!
//! # Secret Redaction
//!
//! By default, sensitive values are replaced with `[REDACTED]`:
//! - Authorization headers (Bearer, Basic)
//! - GitHub tokens (ghp_, gho_, ghr_, ghs_, github_pat_)
//! - Linear API keys (lin_api_)
//! - Generic long hex/base64 strings that look like tokens
//!
//! To reveal secrets for debugging (use with caution):
//! ```bash
//! ISQ_LOG_SECRETS=1 ISQ_LOG=trace isq sync
//! ```

use std::path::PathBuf;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, MakeWriter},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

/// Redact sensitive information from a string.
///
/// Patterns redacted:
/// - `Authorization: Bearer <token>` -> `Authorization: Bearer [REDACTED]`
/// - `Authorization: Basic <token>` -> `Authorization: Basic [REDACTED]`
/// - GitHub tokens (ghp_, gho_, ghr_, ghs_, github_pat_)
/// - Linear API keys (lin_api_)
/// - Long hex strings (40+ chars) that look like tokens
pub fn redact_secrets(input: &str) -> String {
    let mut result = input.to_string();

    // Redact Authorization headers
    // Bearer tokens
    let bearer_re = regex_lite::Regex::new(r"(?i)(Bearer\s+)[A-Za-z0-9_\-\.]+").unwrap();
    result = bearer_re.replace_all(&result, "${1}[REDACTED]").to_string();

    // Basic auth
    let basic_re = regex_lite::Regex::new(r"(?i)(Basic\s+)[A-Za-z0-9+/=]+").unwrap();
    result = basic_re.replace_all(&result, "${1}[REDACTED]").to_string();

    // GitHub tokens: ghp_, gho_, ghr_, ghs_, github_pat_
    let gh_token_re =
        regex_lite::Regex::new(r"(ghp_|gho_|ghr_|ghs_|github_pat_)[A-Za-z0-9_]+").unwrap();
    result = gh_token_re.replace_all(&result, "[REDACTED]").to_string();

    // Linear API keys: lin_api_
    let linear_re = regex_lite::Regex::new(r"lin_api_[A-Za-z0-9_]+").unwrap();
    result = linear_re.replace_all(&result, "[REDACTED]").to_string();

    // Generic long tokens: 40+ character hex or base64-like strings
    // This catches most API tokens without being too aggressive
    let generic_token_re = regex_lite::Regex::new(r"\b[A-Za-z0-9_\-]{40,}\b").unwrap();
    result = generic_token_re
        .replace_all(&result, "[REDACTED]")
        .to_string();

    result
}

/// Check if secret logging is explicitly enabled via ISQ_LOG_SECRETS=1
fn secrets_enabled() -> bool {
    std::env::var("ISQ_LOG_SECRETS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Convert verbosity count to tracing Level.
///
/// - 0: WARN (default)
/// - 1: INFO (-v)
/// - 2: DEBUG (-vv)
/// - 3+: TRACE (-vvv)
pub fn verbosity_to_level(verbosity: u8) -> Level {
    match verbosity {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    }
}

/// Build an EnvFilter from verbosity level, respecting ISQ_LOG override.
fn build_filter(verbosity: u8) -> EnvFilter {
    // ISQ_LOG takes precedence if set
    if let Ok(filter) = std::env::var("ISQ_LOG") {
        return EnvFilter::try_new(&filter).unwrap_or_else(|_| {
            eprintln!(
                "Warning: Invalid ISQ_LOG filter '{}', using default",
                filter
            );
            EnvFilter::new(format!("isq={}", verbosity_to_level(verbosity)))
        });
    }

    // Default: filter to isq crate at specified level
    let level = verbosity_to_level(verbosity);
    EnvFilter::new(format!("isq={}", level))
}

/// A writer that redacts secrets before writing.
struct RedactingWriter<W> {
    inner: W,
    redact: bool,
}

impl<W: std::io::Write> std::io::Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.redact
            && let Ok(s) = std::str::from_utf8(buf)
        {
            let redacted = redact_secrets(s);
            self.inner.write_all(redacted.as_bytes())?;
            return Ok(buf.len());
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// A MakeWriter that wraps another MakeWriter with secret redaction.
struct RedactingMakeWriter<M> {
    inner: M,
    redact: bool,
}

impl<M> RedactingMakeWriter<M> {
    fn new(inner: M, redact: bool) -> Self {
        Self { inner, redact }
    }
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: self.inner.make_writer(),
            redact: self.redact,
        }
    }
}

/// Initialize logging for CLI mode (output to stderr).
///
/// Call this early in main() before any logging occurs.
///
/// # Arguments
/// * `verbosity` - Number of -v flags (0=warn, 1=info, 2=debug, 3+=trace)
pub fn init_cli(verbosity: u8) {
    let filter = build_filter(verbosity);
    let redact = !secrets_enabled();

    let fmt_layer = fmt::layer()
        .with_writer(RedactingMakeWriter::new(std::io::stderr, redact))
        .with_target(verbosity >= 2) // Show module path at debug+
        .with_ansi(atty::is(atty::Stream::Stderr))
        .without_time(); // CLI doesn't need timestamps

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}

/// Initialize logging for daemon mode (output to file with rotation).
///
/// Returns a WorkerGuard that must be held for the lifetime of the daemon
/// to ensure logs are flushed on shutdown.
///
/// Logs are written to `~/.cache/isq/logs/daemon.log` with daily rotation,
/// keeping 7 days of history.
///
/// # Arguments
/// * `verbosity` - Base verbosity level (usually 1 for INFO in daemon mode)
pub fn init_daemon(verbosity: u8) -> Option<WorkerGuard> {
    let log_dir = daemon_log_dir()?;

    // Create log directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("Warning: Failed to create log directory: {}", e);
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(&log_dir, "daemon.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = build_filter(verbosity);
    let redact = !secrets_enabled();

    let fmt_layer = fmt::layer()
        .with_writer(RedactingMakeWriter::new(non_blocking, redact))
        .with_target(true)
        .with_ansi(false); // No ANSI codes in log files

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();

    Some(guard)
}

/// Get the daemon log directory path.
///
/// Returns `~/.cache/isq/logs` on Unix, or platform equivalent.
pub fn daemon_log_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "isq").map(|dirs| dirs.cache_dir().join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_bearer_token() {
        let input = "Authorization: Bearer ghp_1234567890abcdefghij";
        let redacted = redact_secrets(input);
        assert_eq!(redacted, "Authorization: Bearer [REDACTED]");
    }

    #[test]
    fn test_redact_basic_auth() {
        let input = "Authorization: Basic dXNlcjpwYXNz";
        let redacted = redact_secrets(input);
        assert_eq!(redacted, "Authorization: Basic [REDACTED]");
    }

    #[test]
    fn test_redact_github_pat() {
        let input = "token=ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("ghp_"));
    }

    #[test]
    fn test_redact_github_oauth_token() {
        let input = "gho_abcdefghijklmnopqrstuvwxyz123456";
        let redacted = redact_secrets(input);
        assert_eq!(redacted, "[REDACTED]");
    }

    #[test]
    fn test_redact_linear_api_key() {
        let input = "key: lin_api_abcdefghijklmnopqrstuvwxyz";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("lin_api_"));
    }

    #[test]
    fn test_redact_long_token() {
        // 40+ character string that looks like a token
        let input = "secret=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let redacted = redact_secrets(input);
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn test_preserve_short_strings() {
        // Short strings should not be redacted
        let input = "user=alice repo=myrepo";
        let redacted = redact_secrets(input);
        assert_eq!(redacted, input);
    }

    #[test]
    fn test_preserve_normal_text() {
        let input = "Syncing repository camwest/isq...";
        let redacted = redact_secrets(input);
        assert_eq!(redacted, input);
    }

    #[test]
    fn test_verbosity_to_level() {
        assert_eq!(verbosity_to_level(0), Level::WARN);
        assert_eq!(verbosity_to_level(1), Level::INFO);
        assert_eq!(verbosity_to_level(2), Level::DEBUG);
        assert_eq!(verbosity_to_level(3), Level::TRACE);
        assert_eq!(verbosity_to_level(10), Level::TRACE);
    }
}
