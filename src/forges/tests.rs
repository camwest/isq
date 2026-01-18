//! Tests for forge authentication and configuration.

use serial_test::serial;
use std::env;

use super::auth::AuthConfig;
use super::{github, linear};

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

// Test AuthConfig for a mock forge
const TEST_AUTH: AuthConfig = AuthConfig {
    keyring_service: "_isq_test",
    env_var: "_ISQ_TEST_TOKEN",
    cli_command: None,
    display_name: "Test",
    link_command: "isq link test",
};

#[test]
#[serial]
fn test_auth_config_env_var_fallback() {
    let _guard = EnvGuard::set("_ISQ_TEST_TOKEN", "test_token_123");

    let result = TEST_AUTH.get_token();
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "test_token_123");
}

#[test]
#[serial]
fn test_auth_config_has_credentials_with_env_var() {
    let _guard = EnvGuard::set("_ISQ_TEST_TOKEN", "test_token");
    assert!(TEST_AUTH.has_credentials());
}

#[test]
#[serial]
fn test_auth_config_has_credentials_without_anything() {
    let _guard = EnvGuard::unset("_ISQ_TEST_TOKEN");
    // May still be true if keyring has credentials, but shouldn't panic
    let _ = TEST_AUTH.has_credentials();
}

#[test]
#[serial]
fn test_auth_config_error_message() {
    let _guard = EnvGuard::unset("_ISQ_TEST_TOKEN");

    let result = TEST_AUTH.get_token();

    // If it fails (no keyring, no env var), check error message
    if result.is_err() {
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Test not authenticated"));
        assert!(err.contains("isq link test"));
        assert!(err.contains("_ISQ_TEST_TOKEN"));
    }
}

#[test]
fn test_github_auth_config() {
    // Verify GitHub AUTH is properly configured
    assert_eq!(github::AUTH.keyring_service, "github");
    assert_eq!(github::AUTH.env_var, "GITHUB_TOKEN");
    assert!(github::AUTH.cli_command.is_some());
    assert_eq!(github::AUTH.display_name, "GitHub");
}

#[test]
fn test_linear_auth_config() {
    // Verify Linear AUTH is properly configured
    assert_eq!(linear::AUTH.keyring_service, "linear");
    assert_eq!(linear::AUTH.env_var, "LINEAR_API_KEY");
    assert!(linear::AUTH.cli_command.is_none());
    assert_eq!(linear::AUTH.display_name, "Linear");
}

#[test]
#[serial]
fn test_github_token_from_env_var() {
    let _guard = EnvGuard::set("GITHUB_TOKEN", "ghp_test123");

    let result = github::AUTH.get_token();
    // May succeed with env var, or may use gh CLI if available
    if result.is_ok() {
        assert!(!result.unwrap().is_empty());
    }
}

#[test]
#[serial]
fn test_linear_token_from_env_var() {
    let _guard = EnvGuard::set("LINEAR_API_KEY", "lin_test456");

    let result = linear::AUTH.get_token();
    assert!(result.is_ok());
    assert!(!result.unwrap().is_empty());
}
