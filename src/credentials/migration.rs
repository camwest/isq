//! Migration from OS keyring to file-based storage.

use anyhow::Result;

#[cfg(all(not(test), feature = "keyring-migration"))]
use {
    super::Credential,
    super::storage::{CredentialStore, credentials_path, write_store},
    anyhow::anyhow,
    keyring::Entry,
    std::fs::{self, OpenOptions},
    std::io::ErrorKind,
};

#[cfg(all(not(test), feature = "keyring-migration"))]
const SERVICE_NAME: &str = "isq";

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
            remove_keyring_credential(service);
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

/// No-op for test builds
#[cfg(test)]
pub fn migrate_from_keyring() -> Result<()> {
    Ok(())
}
