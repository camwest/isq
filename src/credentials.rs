//! Secure credential storage using OS keyring.
//!
//! Uses the system keychain/credential manager:
//! - macOS: Keychain
//! - Linux: Secret Service (GNOME Keyring, KWallet)
//! - Windows: Credential Manager
//!
//! Falls back to environment variables when keyring is unavailable.

use anyhow::{anyhow, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};

const SERVICE_NAME: &str = "isq";

/// Stored credential with optional refresh token and expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// Store a credential in the OS keyring.
pub fn set_credential(
    service: &str,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<&str>,
) -> Result<()> {
    let credential = Credential {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.map(String::from),
        expires_at: expires_at.map(String::from),
    };

    let json = serde_json::to_string(&credential)?;
    let entry = Entry::new(SERVICE_NAME, service)?;
    entry.set_password(&json).map_err(|e| {
        anyhow!(
            "Failed to store credentials in system keyring: {}\n\n\
            Your system may not have a keyring service running.\n\
            Use environment variables instead:\n\
            - GitHub: export GITHUB_TOKEN=<token>\n\
            - Linear: export LINEAR_API_KEY=<token>",
            e
        )
    })?;

    Ok(())
}

/// Retrieve a credential from the OS keyring.
pub fn get_credential(service: &str) -> Result<Option<Credential>> {
    let entry = match Entry::new(SERVICE_NAME, service) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };

    match entry.get_password() {
        Ok(json) => {
            let credential: Credential = serde_json::from_str(&json)?;
            Ok(Some(credential))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            // Keyring access failed - not a fatal error, just means no stored credential
            // This happens on headless systems without a keyring
            tracing_debug_keyring_error(service, &e);
            Ok(None)
        }
    }
}

/// Remove a credential from the OS keyring.
pub fn remove_credential(service: &str) -> Result<()> {
    let entry = Entry::new(SERVICE_NAME, service)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // Already gone
        Err(e) => Err(anyhow!("Failed to remove credential: {}", e)),
    }
}

/// Check if keyring is available on this system.
pub fn is_keyring_available() -> bool {
    // Try to create an entry - this will fail if no keyring backend is available
    Entry::new(SERVICE_NAME, "_test").is_ok()
}

// Debug helper - we don't have tracing, so this is a no-op for now
fn tracing_debug_keyring_error(_service: &str, _e: &keyring::Error) {
    // In the future, could log: "Keyring access failed for {}: {}"
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a working keyring on the system.
    // They will be skipped in CI environments without one.

    #[test]
    fn test_credential_roundtrip() {
        let test_service = "_isq_test_credential";

        // Try to set a credential - if this fails, keyring isn't available
        match set_credential(
            test_service,
            "test_access_token",
            Some("test_refresh_token"),
            Some("2024-12-31T23:59:59Z"),
        ) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Skipping test: keyring not available ({})", e);
                return;
            }
        }

        // Retrieve
        let cred = get_credential(test_service)
            .expect("Failed to get credential")
            .expect("Credential not found");

        assert_eq!(cred.access_token, "test_access_token");
        assert_eq!(cred.refresh_token, Some("test_refresh_token".to_string()));
        assert_eq!(cred.expires_at, Some("2024-12-31T23:59:59Z".to_string()));

        // Clean up
        let _ = remove_credential(test_service);

        // Verify removal
        let cred = get_credential(test_service).expect("Failed to get credential");
        assert!(cred.is_none());
    }
}
