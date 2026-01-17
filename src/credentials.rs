//! Secure credential storage using file-based storage.
//!
//! Stores credentials in ~/.config/isq/credentials.json with 0600 permissions.
//! This avoids OS keychain permission prompts during background daemon operations.
//!
//! In tests, uses thread-local in-memory storage to avoid file system side effects.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[cfg(not(test))]
use {
    anyhow::anyhow,
    std::collections::HashMap,
    std::fs::{self, OpenOptions},
    std::io::ErrorKind,
    std::path::PathBuf,
};

#[cfg(all(not(test), unix))]
use std::os::unix::fs::PermissionsExt;

#[cfg(all(not(test), feature = "keyring-migration"))]
use keyring::Entry;

#[cfg(all(not(test), feature = "keyring-migration"))]
const SERVICE_NAME: &str = "isq";

#[cfg(not(test))]
const CREDENTIALS_FILE: &str = "credentials.json";

/// Thread-local in-memory credential store for tests.
/// Avoids file system side effects during test runs.
#[cfg(test)]
mod mock_store {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static STORE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }

    pub fn set(key: &str, value: &str) {
        STORE.with(|s| s.borrow_mut().insert(key.to_string(), value.to_string()));
    }

    pub fn get(key: &str) -> Option<String> {
        STORE.with(|s| s.borrow().get(key).cloned())
    }

    pub fn remove(key: &str) {
        STORE.with(|s| s.borrow_mut().remove(key));
    }
}

/// Stored credential with optional refresh token and expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// All stored credentials, keyed by service name (github, linear, jira).
#[cfg(not(test))]
type CredentialStore = HashMap<String, Credential>;

/// Get the credentials file path: ~/.config/isq/credentials.json
#[cfg(not(test))]
fn credentials_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow!("Could not determine config directory"))?;
    Ok(dirs.config_dir().join(CREDENTIALS_FILE))
}

/// Read the credential store from disk
#[cfg(not(test))]
fn read_store() -> Result<CredentialStore> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(CredentialStore::default());
    }
    let content = fs::read_to_string(&path)?;
    let store: CredentialStore = serde_json::from_str(&content)?;
    Ok(store)
}

/// Write the credential store to disk with 0600 permissions.
///
/// - Unix: Set restrictive permissions (owner read/write only)
/// - Windows: Inherits user-private ACLs from config directory (secure by default)
#[cfg(not(test))]
fn write_store(store: &CredentialStore) -> Result<()> {
    let path = credentials_path()?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(store)?;

    // Write to temp file first, then rename for atomicity
    let temp_path = path.with_extension("json.tmp");

    // Clean up any stale temp file from a previous failed write
    let _ = fs::remove_file(&temp_path);

    if let Err(e) = fs::write(&temp_path, &json) {
        let _ = fs::remove_file(&temp_path);
        return Err(e.into());
    }

    #[cfg(unix)]
    if let Err(e) = fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(&temp_path);
        return Err(e.into());
    }

    if let Err(e) = fs::rename(&temp_path, &path) {
        let _ = fs::remove_file(&temp_path);
        return Err(e.into());
    }

    Ok(())
}

/// Store a credential in the file store (or mock in tests).
#[cfg(not(test))]
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

    let mut store = read_store()?;
    store.insert(service.to_string(), credential);
    write_store(&store)?;
    Ok(())
}

/// Store a credential in the mock store (test version).
#[cfg(test)]
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
    mock_store::set(service, &json);
    Ok(())
}

/// Retrieve a credential from the file store.
#[cfg(not(test))]
pub fn get_credential(service: &str) -> Result<Option<Credential>> {
    let store = read_store()?;
    Ok(store.get(service).cloned())
}

/// Retrieve a credential from the mock store (test version).
#[cfg(test)]
pub fn get_credential(service: &str) -> Result<Option<Credential>> {
    match mock_store::get(service) {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

/// Remove a credential from the file store.
#[cfg(not(test))]
pub fn remove_credential(service: &str) -> Result<()> {
    let mut store = read_store()?;
    store.remove(service);
    write_store(&store)?;
    Ok(())
}

/// Remove a credential from the mock store (test version).
#[cfg(test)]
pub fn remove_credential(service: &str) -> Result<()> {
    mock_store::remove(service);
    Ok(())
}

// === Migration from keyring ===

/// Services that may have credentials stored in the OS keyring.
#[cfg(all(not(test), feature = "keyring-migration"))]
const KEYRING_SERVICES: &[&str] = &["github", "linear", "jira"];

/// Migrate credentials from OS keyring to file storage.
///
/// Runs once on first use and cleans up old keyring entries.
/// Uses a lock file to prevent race conditions when multiple processes start simultaneously.
/// Only available when compiled with keyring-migration feature.
#[cfg(all(not(test), feature = "keyring-migration"))]
pub fn migrate_from_keyring() -> Result<()> {
    let path = credentials_path()?;

    // Skip if credentials file already exists (migration done)
    if path.exists() {
        return Ok(());
    }

    // Create parent directory if needed
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Use a lock file to prevent race conditions during migration
    let lock_path = path.with_extension("json.migrating");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(_) => {
            // We got the lock, proceed with migration
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            // Another process is migrating, skip
            return Ok(());
        }
        Err(e) => {
            return Err(anyhow!("Failed to create migration lock: {}", e));
        }
    }

    // Double-check file doesn't exist (another process may have finished between our checks)
    if path.exists() {
        let _ = fs::remove_file(&lock_path);
        return Ok(());
    }

    let mut store = CredentialStore::default();
    let mut migrated_any = false;

    for service in KEYRING_SERVICES {
        if let Some(credential) = get_keyring_credential(service) {
            store.insert(service.to_string(), credential);
            migrated_any = true;
        }
    }

    if migrated_any {
        write_store(&store)?;

        // Clean up old keyring entries after successful migration
        for service in KEYRING_SERVICES {
            let _ = remove_keyring_credential(service);
        }

        eprintln!(
            "Migrated credentials from system keychain to {}",
            path.display()
        );
    }

    // Clean up lock file
    let _ = fs::remove_file(&lock_path);

    Ok(())
}

/// No-op when keyring-migration feature is disabled
#[cfg(all(not(test), not(feature = "keyring-migration")))]
pub fn migrate_from_keyring() -> Result<()> {
    Ok(())
}

/// Get a credential from the OS keyring (for migration)
#[cfg(all(not(test), feature = "keyring-migration"))]
fn get_keyring_credential(service: &str) -> Option<Credential> {
    let entry = match Entry::new(SERVICE_NAME, service) {
        Ok(e) => e,
        Err(_) => return None,
    };

    match entry.get_password() {
        Ok(json) => serde_json::from_str(&json).ok(),
        Err(_) => None,
    }
}

/// Remove a credential from the OS keyring (cleanup after migration)
#[cfg(all(not(test), feature = "keyring-migration"))]
fn remove_keyring_credential(service: &str) {
    if let Ok(entry) = Entry::new(SERVICE_NAME, service) {
        let _ = entry.delete_credential();
    }
}

// No-op for test builds
#[cfg(test)]
pub fn migrate_from_keyring() -> Result<()> {
    Ok(())
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

    // === Mock store tests ===

    #[test]
    fn test_credential_roundtrip() {
        let test_service = "_isq_test_credential";

        // Set a credential
        set_credential(
            test_service,
            "test_access_token",
            Some("test_refresh_token"),
            Some("2024-12-31T23:59:59Z"),
        )
        .unwrap();

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
    fn test_credential_minimal() {
        let test_service = "_isq_test_minimal";

        // Set a credential with only access token
        set_credential(test_service, "minimal_token", None, None).unwrap();

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
        assert!(result.unwrap().is_none());
    }

    // === File format tests ===

    #[test]
    fn test_store_serialization() {
        use std::collections::HashMap;

        let mut store: HashMap<String, Credential> = HashMap::new();
        store.insert(
            "github".to_string(),
            Credential {
                access_token: "ghp_xxx".to_string(),
                refresh_token: None,
                expires_at: None,
            },
        );
        store.insert(
            "linear".to_string(),
            Credential {
                access_token: "lin_api_xxx".to_string(),
                refresh_token: None,
                expires_at: None,
            },
        );

        let json = serde_json::to_string_pretty(&store).unwrap();
        assert!(json.contains("github"));
        assert!(json.contains("linear"));
        assert!(json.contains("ghp_xxx"));
        assert!(json.contains("lin_api_xxx"));
    }

    #[test]
    fn test_store_deserialization() {
        use std::collections::HashMap;

        let json = r#"{
            "github": {
                "access_token": "ghp_xxx",
                "refresh_token": null,
                "expires_at": null
            },
            "linear": {
                "access_token": "lin_api_xxx"
            }
        }"#;

        let store: HashMap<String, Credential> = serde_json::from_str(json).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.get("github").unwrap().access_token, "ghp_xxx");
        assert_eq!(store.get("linear").unwrap().access_token, "lin_api_xxx");
    }
}
