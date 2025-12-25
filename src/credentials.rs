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

    // === Serialization tests (don't require keyring) ===

    #[test]
    fn test_credential_serialization_full() {
        let cred = Credential {
            access_token: "ghp_abc123".to_string(),
            refresh_token: Some("ghr_xyz789".to_string()),
            expires_at: Some("2024-12-31T23:59:59Z".to_string()),
        };

        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("ghp_abc123"));
        assert!(json.contains("ghr_xyz789"));
        assert!(json.contains("2024-12-31T23:59:59Z"));
    }

    #[test]
    fn test_credential_serialization_minimal() {
        let cred = Credential {
            access_token: "token123".to_string(),
            refresh_token: None,
            expires_at: None,
        };

        let json = serde_json::to_string(&cred).unwrap();
        assert!(json.contains("token123"));
        // Optional fields should be omitted when None
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("expires_at"));
    }

    #[test]
    fn test_credential_deserialization_full() {
        let json = r#"{"access_token":"abc","refresh_token":"xyz","expires_at":"2024-01-01"}"#;
        let cred: Credential = serde_json::from_str(json).unwrap();

        assert_eq!(cred.access_token, "abc");
        assert_eq!(cred.refresh_token, Some("xyz".to_string()));
        assert_eq!(cred.expires_at, Some("2024-01-01".to_string()));
    }

    #[test]
    fn test_credential_deserialization_minimal() {
        let json = r#"{"access_token":"token_only"}"#;
        let cred: Credential = serde_json::from_str(json).unwrap();

        assert_eq!(cred.access_token, "token_only");
        assert_eq!(cred.refresh_token, None);
        assert_eq!(cred.expires_at, None);
    }

    #[test]
    fn test_credential_deserialization_with_null_fields() {
        let json = r#"{"access_token":"tok","refresh_token":null,"expires_at":null}"#;
        let cred: Credential = serde_json::from_str(json).unwrap();

        assert_eq!(cred.access_token, "tok");
        assert_eq!(cred.refresh_token, None);
        assert_eq!(cred.expires_at, None);
    }

    #[test]
    fn test_credential_roundtrip_serialization() {
        let original = Credential {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some("2025-06-15T12:00:00Z".to_string()),
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: Credential = serde_json::from_str(&json).unwrap();

        assert_eq!(original.access_token, restored.access_token);
        assert_eq!(original.refresh_token, restored.refresh_token);
        assert_eq!(original.expires_at, restored.expires_at);
    }

    // === Keyring integration tests (skipped if keyring unavailable) ===

    #[test]
    fn test_credential_keyring_roundtrip() {
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

    #[test]
    fn test_credential_keyring_minimal() {
        let test_service = "_isq_test_minimal";

        // Try to set a credential with only access token
        match set_credential(test_service, "minimal_token", None, None) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Skipping test: keyring not available ({})", e);
                return;
            }
        }

        let cred = get_credential(test_service)
            .expect("Failed to get credential")
            .expect("Credential not found");

        assert_eq!(cred.access_token, "minimal_token");
        assert_eq!(cred.refresh_token, None);
        assert_eq!(cred.expires_at, None);

        // Clean up
        let _ = remove_credential(test_service);
    }

    #[test]
    fn test_get_nonexistent_credential() {
        // Getting a credential that doesn't exist should return None, not error
        let result = get_credential("_isq_definitely_does_not_exist_xyz123");
        assert!(result.is_ok());
        // May be None (not found) or Some (if leftover from previous test)
        // The important thing is it doesn't error
    }
}
