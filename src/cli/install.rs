//! Install management CLI commands
//!
//! These commands are internal/hidden and used by installer scripts.

use anyhow::Result;
use chrono::Utc;
use std::path::PathBuf;

use crate::install::{self, InstallMethod, InstallReceipt};

/// Write install receipt (called by installation scripts)
pub fn cmd_write_receipt(method: String, binary_path: PathBuf, auto_update: bool) -> Result<()> {
    let install_method: InstallMethod = method.parse()?;

    let receipt = InstallReceipt {
        version: env!("CARGO_PKG_VERSION").to_string(),
        install_method,
        installed_at: Utc::now(),
        binary_path,
        auto_update,
    };

    if install::write_receipt(&receipt)? {
        println!("Install receipt written");
    } else {
        println!("Install receipt already exists, not overwriting");
    }

    Ok(())
}
