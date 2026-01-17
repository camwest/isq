//! Install receipt tracking for isq
//!
//! Tracks how isq was installed and when, enabling auto-update workflows.
//! The receipt file is written once during installation and should not be
//! modified afterward to preserve original install metadata.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
/// File is created atomically with 0600 permissions on Unix systems.
pub fn write_receipt(receipt: &InstallReceipt) -> Result<bool> {
    let path = receipt_path()?;

    // Create config directory if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_string_pretty(receipt)?;

    // Use create_new for atomic check-and-create, with 0600 permissions on Unix
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let file = OpenOptions::new()
            .write(true)
            .create_new(true) // Atomic: fails if file exists
            .mode(0o600) // Set permissions at creation time
            .open(&path);

        match file {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    #[cfg(not(unix))]
    {
        use std::fs::OpenOptions;
        use std::io::Write;

        let file = OpenOptions::new()
            .write(true)
            .create_new(true) // Atomic: fails if file exists
            .open(&path);

        match file {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

/// Detect how isq was installed.
///
/// Checks the install receipt first. If no receipt exists (e.g., users who
/// installed before the receipt system, or package manager installs), falls
/// back to detecting the install method from the binary path.
pub fn detect_install_method() -> InstallMethod {
    // 1. Check receipt first
    if let Ok(Some(receipt)) = read_receipt() {
        return receipt.install_method;
    }

    // 2. Fall back to path detection
    detect_from_binary_path()
}

/// Detect install method from the current binary's path.
fn detect_from_binary_path() -> InstallMethod {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return InstallMethod::Unknown,
    };

    // Resolve symlinks - Homebrew symlinks from /usr/local/bin to Cellar
    let resolved = std::fs::canonicalize(&exe).unwrap_or(exe);
    detect_from_path(&resolved)
}

/// Detect install method from a given path.
///
/// This is separated from `detect_from_binary_path` to enable unit testing
/// with mock paths.
fn detect_from_path(path: &Path) -> InstallMethod {
    let path_str = path.to_string_lossy();

    // Homebrew: /opt/homebrew/Cellar/isq/... (macOS ARM)
    //           /usr/local/Cellar/isq/... (macOS Intel)
    //           /home/linuxbrew/.linuxbrew/Cellar/isq/... (Linux)
    if path_str.contains("/Cellar/isq/") {
        return InstallMethod::Homebrew;
    }

    // Cargo: ~/.cargo/bin/isq
    if path_str.contains("/.cargo/bin/isq") {
        return InstallMethod::Cargo;
    }

    #[cfg(target_os = "windows")]
    {
        let path_lower = path_str.to_lowercase();

        // Scoop: C:\Users\<user>\scoop\apps\isq\...
        if path_lower.contains("\\scoop\\apps\\isq\\") {
            return InstallMethod::Scoop;
        }

        // Cargo: C:\Users\<user>\.cargo\bin\isq.exe
        if path_lower.contains("\\.cargo\\bin\\isq") {
            return InstallMethod::Cargo;
        }
    }

    InstallMethod::Unknown
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

    // === Path detection tests ===

    #[test]
    fn test_detect_from_path() {
        let cases = [
            // Homebrew paths
            ("/opt/homebrew/Cellar/isq/0.1.0/bin/isq", InstallMethod::Homebrew),
            ("/usr/local/Cellar/isq/0.1.0/bin/isq", InstallMethod::Homebrew),
            ("/home/linuxbrew/.linuxbrew/Cellar/isq/0.1.0/bin/isq", InstallMethod::Homebrew),
            // Cargo paths
            ("/Users/cam/.cargo/bin/isq", InstallMethod::Cargo),
            ("/home/cam/.cargo/bin/isq", InstallMethod::Cargo),
            // Unknown paths
            ("/some/random/path/isq", InstallMethod::Unknown),
            ("/Users/cam/src/isq/target/debug/isq", InstallMethod::Unknown),
            ("/usr/local/bin/isq", InstallMethod::Unknown),
        ];

        for (path, expected) in cases {
            assert_eq!(
                detect_from_path(Path::new(path)),
                expected,
                "path: {}",
                path
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_detect_from_path_windows() {
        let cases = [
            (r"C:\Users\cam\scoop\apps\isq\current\isq.exe", InstallMethod::Scoop),
            (r"C:\Users\Cam\Scoop\Apps\isq\current\isq.exe", InstallMethod::Scoop),
            (r"C:\Users\cam\.cargo\bin\isq.exe", InstallMethod::Cargo),
        ];

        for (path, expected) in cases {
            assert_eq!(
                detect_from_path(Path::new(path)),
                expected,
                "path: {}",
                path
            );
        }
    }

    #[test]
    fn test_detect_from_binary_path_does_not_panic() {
        // Integration test: verify the function runs without panicking
        let _ = detect_from_binary_path();
    }
}
