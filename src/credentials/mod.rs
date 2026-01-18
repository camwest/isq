//! Secure credential storage using file-based storage.
//!
//! Stores credentials in ~/.config/isq/credentials.json with 0600 permissions.
//! This avoids OS keychain permission prompts during background daemon operations.
//!
//! In tests, uses thread-local in-memory storage to avoid file system side effects.

mod migration;
mod storage;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};

// Re-export public API
pub use migration::migrate_from_keyring;
pub use storage::{get_credential, remove_credential, set_credential};

/// Stored credential with optional refresh token and expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}
