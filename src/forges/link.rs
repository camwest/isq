//! Link operation types for connecting repos to forges.

use anyhow::{Result, anyhow};

use super::forge_type::ALL_FORGE_TYPES;

/// Arguments for the link command, parsed from CLI options.
///
/// This is a generic options container that each forge interprets according
/// to its own schema. This allows forge-specific options without coupling
/// the shared code to any particular forge.
///
/// Examples:
/// - Linear: `-o team=ENG` or `-o list-teams`
/// - JIRA: `-o project=PROJ`, `-o site=NAME`, or `-o list-projects`
/// - GitHub: (no options currently)
#[derive(Debug, Clone, Default)]
pub struct LinkArgs {
    /// Key-value options (e.g., team=ENG, project=PROJ)
    options: std::collections::HashMap<String, String>,
    /// Flag options (e.g., list-teams, list-projects)
    flags: std::collections::HashSet<String>,
}

impl LinkArgs {
    /// Parse from CLI -o key=value options
    pub fn parse(opts: &[String]) -> Result<Self> {
        let mut args = Self::default();
        for opt in opts {
            if let Some((key, value)) = opt.split_once('=') {
                args.options.insert(key.to_string(), value.to_string());
            } else {
                // Treat as a flag (e.g., "list-teams", "list-projects")
                args.flags.insert(opt.to_string());
            }
        }
        Ok(args)
    }

    /// Get an option value by key
    pub fn get(&self, key: &str) -> Option<&str> {
        self.options.get(key).map(|s| s.as_str())
    }

    /// Check if a flag is set
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.contains(flag)
    }
}

/// Result of a successful link operation
#[derive(Debug, Clone)]
pub struct LinkResult {
    pub display_name: String,
}

/// Generate error message for repos not linked to a forge
pub fn not_linked_error() -> anyhow::Error {
    let forges: Vec<_> = ALL_FORGE_TYPES
        .iter()
        .map(|f| format!("  isq link {}", f.as_str()))
        .collect();
    anyhow!(
        "This repo is not linked to an issue tracker.\n\nRun one of:\n{}",
        forges.join("\n")
    )
}
