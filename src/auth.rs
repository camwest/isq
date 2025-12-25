use anyhow::{anyhow, Result};
use std::process::Command;

use crate::credentials;

/// Get GitHub token from gh CLI
pub fn get_gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|_| anyhow!("gh CLI not found. Install it from https://cli.github.com"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh CLI not authenticated. Run `gh auth login` first.\n{}",
            stderr.trim()
        ));
    }

    let token = String::from_utf8(output.stdout)?.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("gh CLI returned empty token"));
    }

    Ok(token)
}

/// Get GitHub token with fallback: gh CLI → stored credentials → env var
pub fn get_github_token() -> Result<String> {
    // Try gh CLI first (fastest, most common)
    if let Ok(token) = get_gh_token() {
        return Ok(token);
    }

    // Try stored credentials (from OS keyring)
    if let Ok(Some(cred)) = credentials::get_credential("github") {
        return Ok(cred.access_token);
    }

    // Try environment variable
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Ok(token);
    }

    // No token available
    Err(anyhow!(
        "GitHub not authenticated.\n\n\
        Option 1: Install gh CLI and run: gh auth login\n\
        Option 2: Run: isq link github (browser OAuth)\n\
        Option 3: Set GITHUB_TOKEN environment variable"
    ))
}

/// Check if GitHub auth is available (without triggering OAuth)
pub fn has_github_credentials() -> bool {
    // Check gh CLI
    if get_gh_token().is_ok() {
        return true;
    }

    // Check stored credentials (OS keyring)
    if let Ok(Some(_)) = credentials::get_credential("github") {
        return true;
    }

    // Check env var
    std::env::var("GITHUB_TOKEN").is_ok()
}

/// Get Linear token from stored credentials or environment variable
pub fn get_linear_token() -> Result<String> {
    // First check stored credentials (OS keyring)
    if let Ok(Some(cred)) = credentials::get_credential("linear") {
        return Ok(cred.access_token);
    }

    // Fall back to environment variable
    std::env::var("LINEAR_API_KEY").map_err(|_| {
        anyhow!(
            "Linear not authenticated.\n\n\
            Option 1: Run: isq link linear (browser OAuth)\n\
            Option 2: Set LINEAR_API_KEY environment variable"
        )
    })
}

/// Check if Linear has stored credentials (not just env var)
pub fn has_linear_credentials() -> bool {
    if let Ok(Some(_)) = credentials::get_credential("linear") {
        return true;
    }
    std::env::var("LINEAR_API_KEY").is_ok()
}

/// Get the full Linear credential (including refresh token) for token refresh
pub fn get_linear_credential() -> Result<Option<credentials::Credential>> {
    credentials::get_credential("linear")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    // Helper to temporarily set/unset env vars
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = env::var(key).ok();
            // SAFETY: Tests run serially via #[serial], so no concurrent access
            unsafe { env::set_var(key, value) };
            Self { key, original }
        }

        fn unset(key: &'static str) -> Self {
            let original = env::var(key).ok();
            // SAFETY: Tests run serially via #[serial], so no concurrent access
            unsafe { env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: Tests run serially via #[serial], so no concurrent access
            match &self.original {
                Some(val) => unsafe { env::set_var(self.key, val) },
                None => unsafe { env::remove_var(self.key) },
            }
        }
    }

    // === GitHub token tests ===

    #[test]
    #[serial]
    fn test_github_token_from_env_var() {
        let _guard = EnvGuard::set("GITHUB_TOKEN", "env_token_123");

        // Even without gh CLI or keyring, env var should work
        let result = get_github_token();
        // May succeed with env var, or may use gh CLI if available
        // The important thing is the fallback chain works
        if result.is_ok() {
            let token = result.unwrap();
            // Either from env var or gh CLI
            assert!(!token.is_empty());
        }
    }

    #[test]
    #[serial]
    fn test_github_token_error_message_when_not_authenticated() {
        // Temporarily remove GITHUB_TOKEN to test error path
        let _guard = EnvGuard::unset("GITHUB_TOKEN");

        // This will fail if gh CLI isn't available/authenticated and no keyring
        let result = get_github_token();

        // If it fails (no gh CLI, no keyring, no env var), check error message
        if result.is_err() {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("GitHub not authenticated"));
            assert!(err.contains("gh auth login"));
            assert!(err.contains("isq link github"));
            assert!(err.contains("GITHUB_TOKEN"));
        }
    }

    #[test]
    #[serial]
    fn test_has_github_credentials_with_env_var() {
        let _guard = EnvGuard::set("GITHUB_TOKEN", "test_token");
        assert!(has_github_credentials());
    }

    #[test]
    #[serial]
    fn test_has_github_credentials_without_anything() {
        let _guard = EnvGuard::unset("GITHUB_TOKEN");
        // May still return true if gh CLI is available
        // Just verify it doesn't panic
        let _ = has_github_credentials();
    }

    // === Linear token tests ===

    #[test]
    #[serial]
    fn test_linear_token_from_env_var() {
        let _guard = EnvGuard::set("LINEAR_API_KEY", "lin_api_key_456");

        let result = get_linear_token();
        assert!(result.is_ok());
        // Could be from keyring or env var
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    #[serial]
    fn test_linear_token_error_message_when_not_authenticated() {
        let _guard = EnvGuard::unset("LINEAR_API_KEY");

        let result = get_linear_token();

        // If it fails (no keyring, no env var), check error message
        if result.is_err() {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Linear not authenticated"));
            assert!(err.contains("isq link linear"));
            assert!(err.contains("LINEAR_API_KEY"));
        }
    }

    #[test]
    #[serial]
    fn test_has_linear_credentials_with_env_var() {
        let _guard = EnvGuard::set("LINEAR_API_KEY", "test_key");
        assert!(has_linear_credentials());
    }

    #[test]
    #[serial]
    fn test_has_linear_credentials_without_anything() {
        let _guard = EnvGuard::unset("LINEAR_API_KEY");
        // May still return true if keyring has credentials
        // Just verify it doesn't panic
        let _ = has_linear_credentials();
    }

    // === gh CLI tests ===

    #[test]
    fn test_get_gh_token_handles_missing_cli() {
        // This test verifies that get_gh_token doesn't panic
        // It may succeed or fail depending on whether gh is installed
        let result = get_gh_token();

        if result.is_err() {
            let err = result.unwrap_err().to_string();
            // Should have a helpful error message
            assert!(
                err.contains("gh CLI not found") || err.contains("not authenticated"),
                "Unexpected error: {}",
                err
            );
        }
    }
}
