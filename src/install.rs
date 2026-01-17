//! Install receipt tracking for isq
//!
//! Tracks how isq was installed and when, enabling auto-update workflows.
//! The receipt file is written once during installation and should not be
//! modified afterward to preserve original install metadata.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::user_config;

/// How isq was installed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InstallMethod {
    Standalone,
    Homebrew,
    Scoop,
    Cargo,
    Unknown,
}

impl Default for InstallMethod {
    fn default() -> Self {
        InstallMethod::Unknown
    }
}

impl std::str::FromStr for InstallMethod {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "standalone" => Ok(InstallMethod::Standalone),
            "homebrew" => Ok(InstallMethod::Homebrew),
            "scoop" => Ok(InstallMethod::Scoop),
            "cargo" => Ok(InstallMethod::Cargo),
            "unknown" => Ok(InstallMethod::Unknown),
            _ => Err(anyhow::anyhow!(
                "Unknown install method: '{}'. Valid options: standalone, homebrew, scoop, cargo",
                s
            )),
        }
    }
}

/// Installation receipt - written once during installation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReceipt {
    /// isq version at install time
    pub version: String,
    /// How isq was installed
    pub install_method: InstallMethod,
    /// When isq was installed
    pub installed_at: DateTime<Utc>,
    /// Path to the isq binary
    pub binary_path: PathBuf,
    /// Whether auto-update is enabled
    #[serde(default)]
    pub auto_update: bool,
}

/// Get path to install receipt file (~/.config/isq/install.json)
pub fn receipt_path() -> Result<PathBuf> {
    Ok(user_config::config_dir()?.join("install.json"))
}

/// Read install receipt, returning None if missing or corrupted
///
/// This function handles errors gracefully:
/// - Missing file: Returns Ok(None)
/// - Corrupted/invalid JSON: Logs warning, returns Ok(None)
/// - Other I/O errors: Logs warning, returns Ok(None)
pub fn read_receipt() -> Result<Option<InstallReceipt>> {
    let path = receipt_path()?;

    if !path.exists() {
        return Ok(None);
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: Failed to read install receipt: {}", e);
            return Ok(None);
        }
    };

    match serde_json::from_str(&content) {
        Ok(receipt) => Ok(Some(receipt)),
        Err(e) => {
            eprintln!("Warning: Install receipt corrupted, ignoring: {}", e);
            Ok(None)
        }
    }
}

/// Write install receipt (only if one doesn't already exist)
///
/// Returns Ok(true) if written, Ok(false) if receipt already exists.
/// File is written with 0600 permissions on Unix systems.
pub fn write_receipt(receipt: &InstallReceipt) -> Result<bool> {
    let path = receipt_path()?;

    // Don't overwrite existing receipt - preserve original install metadata
    if path.exists() {
        return Ok(false);
    }

    // Create config directory if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(receipt)?;

    // Write with restricted permissions on Unix
    #[cfg(unix)]
    {
        use std::fs::{File, Permissions};
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let mut file = File::create(&path)?;
        file.write_all(content.as_bytes())?;
        file.set_permissions(Permissions::from_mode(0o600))?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(&path, content)?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_method_from_str() {
        assert_eq!(
            "standalone".parse::<InstallMethod>().unwrap(),
            InstallMethod::Standalone
        );
        assert_eq!(
            "homebrew".parse::<InstallMethod>().unwrap(),
            InstallMethod::Homebrew
        );
        assert_eq!(
            "scoop".parse::<InstallMethod>().unwrap(),
            InstallMethod::Scoop
        );
        assert_eq!(
            "cargo".parse::<InstallMethod>().unwrap(),
            InstallMethod::Cargo
        );
        assert_eq!(
            "unknown".parse::<InstallMethod>().unwrap(),
            InstallMethod::Unknown
        );
        // Case insensitive
        assert_eq!(
            "STANDALONE".parse::<InstallMethod>().unwrap(),
            InstallMethod::Standalone
        );
        assert_eq!(
            "Homebrew".parse::<InstallMethod>().unwrap(),
            InstallMethod::Homebrew
        );
    }

    #[test]
    fn test_install_method_from_str_invalid() {
        let result = "invalid".parse::<InstallMethod>();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Unknown install method"));
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn test_install_method_default() {
        assert_eq!(InstallMethod::default(), InstallMethod::Unknown);
    }

    #[test]
    fn test_install_method_serialization() {
        let methods = vec![
            (InstallMethod::Standalone, "\"standalone\""),
            (InstallMethod::Homebrew, "\"homebrew\""),
            (InstallMethod::Scoop, "\"scoop\""),
            (InstallMethod::Cargo, "\"cargo\""),
            (InstallMethod::Unknown, "\"unknown\""),
        ];

        for (method, expected) in methods {
            let json = serde_json::to_string(&method).unwrap();
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn test_receipt_roundtrip() {
        let receipt = InstallReceipt {
            version: "0.1.0".to_string(),
            install_method: InstallMethod::Standalone,
            installed_at: Utc::now(),
            binary_path: PathBuf::from("/usr/local/bin/isq"),
            auto_update: true,
        };

        let json = serde_json::to_string_pretty(&receipt).unwrap();
        let parsed: InstallReceipt = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, receipt.version);
        assert_eq!(parsed.install_method, receipt.install_method);
        assert_eq!(parsed.binary_path, receipt.binary_path);
        assert_eq!(parsed.auto_update, receipt.auto_update);
    }

    #[test]
    fn test_receipt_auto_update_defaults_false() {
        // JSON without auto_update field should default to false
        let json = r#"{
            "version": "0.1.0",
            "install_method": "standalone",
            "installed_at": "2025-01-17T10:30:00Z",
            "binary_path": "/usr/local/bin/isq"
        }"#;

        let receipt: InstallReceipt = serde_json::from_str(json).unwrap();
        assert!(!receipt.auto_update);
    }

    #[test]
    fn test_parse_invalid_json() {
        let invalid_json = "{ not valid json }";
        let result: Result<InstallReceipt, _> = serde_json::from_str(invalid_json);
        assert!(result.is_err());
    }
}
