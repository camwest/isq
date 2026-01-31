//! File-based credential storage operations.

use super::Credential;
use anyhow::Result;

#[cfg(not(test))]
use {
    anyhow::anyhow, std::collections::HashMap, std::fs, std::os::unix::fs::PermissionsExt,
    std::path::PathBuf,
};

#[cfg(not(test))]
const CREDENTIALS_FILE: &str = "credentials.json";

/// All stored credentials, keyed by service name (github, linear, jira).
#[cfg(not(test))]
pub(crate) type CredentialStore = HashMap<String, Credential>;

/// Get the credentials file path: ~/.config/isq/credentials.json
#[cfg(not(test))]
pub(crate) fn credentials_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow!("Could not determine config directory"))?;
    Ok(dirs.config_dir().join(CREDENTIALS_FILE))
}

/// Read the credential store from disk
#[cfg(not(test))]
pub(crate) fn read_store() -> Result<CredentialStore> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(CredentialStore::default());
    }
    let content = fs::read_to_string(&path)?;
    let store: CredentialStore = serde_json::from_str(&content)?;
    Ok(store)
}

/// Write the credential store to disk with 0600 permissions (owner read/write only).
#[cfg(not(test))]
pub(crate) fn write_store(store: &CredentialStore) -> Result<()> {
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

/// Retrieve a credential from the file store.
#[cfg(not(test))]
pub fn get_credential(service: &str) -> Result<Option<Credential>> {
    let store = read_store()?;
    Ok(store.get(service).cloned())
}

/// Remove a credential from the file store.
#[cfg(not(test))]
pub fn remove_credential(service: &str) -> Result<()> {
    let mut store = read_store()?;
    store.remove(service);
    write_store(&store)?;
    Ok(())
}

// === Test mock implementations ===

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

#[cfg(test)]
pub fn get_credential(service: &str) -> Result<Option<Credential>> {
    match mock_store::get(service) {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
pub fn remove_credential(service: &str) -> Result<()> {
    mock_store::remove(service);
    Ok(())
}

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
