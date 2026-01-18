//! Version display with install method information

use crate::install::{InstallMethod, detect_install_method, read_receipt};

/// Format version string for a given install method
fn format_version(version: &str, method: &InstallMethod, auto_update: bool) -> String {
    match method {
        InstallMethod::Standalone if auto_update => {
            format!("isq {} (standalone, auto-updates enabled)", version)
        }
        InstallMethod::Standalone => format!("isq {} (standalone)", version),
        InstallMethod::Homebrew => {
            format!(
                "isq {} (homebrew)\nNote: Run `brew upgrade isq` to update.",
                version
            )
        }
        InstallMethod::Scoop => {
            format!(
                "isq {} (scoop)\nNote: Run `scoop update isq` to update.",
                version
            )
        }
        InstallMethod::Cargo => {
            format!(
                "isq {} (cargo)\nNote: Run `cargo install isq` to update.",
                version
            )
        }
        InstallMethod::Unknown => format!("isq {}", version),
    }
}

/// Print version info with install method and update instructions
pub fn print_version() {
    let version = env!("CARGO_PKG_VERSION");
    let method = detect_install_method();
    let auto_update = read_receipt()
        .ok()
        .flatten()
        .map(|r| r.auto_update)
        .unwrap_or(false);

    println!("{}", format_version(version, &method, auto_update));
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = env!("CARGO_PKG_VERSION");

    #[test]
    fn test_standalone_with_auto_update() {
        let output = format_version(&VERSION, &InstallMethod::Standalone, true);
        assert!(output.contains("standalone"));
        assert!(output.contains("auto-updates enabled"));
    }

    #[test]
    fn test_standalone_without_auto_update() {
        let output = format_version(&VERSION, &InstallMethod::Standalone, false);
        assert!(output.contains("standalone"));
        assert!(!output.contains("auto-updates enabled"));
    }

    #[test]
    fn test_homebrew_output() {
        let output = format_version(&VERSION, &InstallMethod::Homebrew, false);
        assert!(output.contains("homebrew"));
        assert!(output.contains("brew upgrade isq"));
    }

    #[test]
    fn test_scoop_output() {
        let output = format_version(&VERSION, &InstallMethod::Scoop, false);
        assert!(output.contains("scoop"));
        assert!(output.contains("scoop update isq"));
    }

    #[test]
    fn test_cargo_output() {
        let output = format_version(&VERSION, &InstallMethod::Cargo, false);
        assert!(output.contains("cargo"));
        assert!(output.contains("cargo install isq"));
    }

    #[test]
    fn test_unknown_minimal_output() {
        let output = format_version(&VERSION, &InstallMethod::Unknown, false);
        assert_eq!(output, format!("isq {}", VERSION));
        assert!(!output.contains('('));
        assert!(!output.contains("Note:"));
    }
}
