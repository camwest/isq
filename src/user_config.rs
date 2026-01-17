//! Global user configuration from ~/.config/isq/config.toml
//!
//! This module handles user-level settings that apply across all repositories:
//! - Custom views (named filter combinations)
//! - Default settings (json output, sort order, etc.)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Global user configuration
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub views: HashMap<String, View>,
}

/// Default settings applied when flags aren't explicitly provided
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Default JSON output (default: false)
    #[serde(default)]
    pub json: bool,
    /// Default quiet mode - suppress success messages (default: false)
    #[serde(default)]
    pub quiet: bool,
    /// Default sort order
    pub sort: Option<String>,
    /// Default state filter
    pub state: Option<String>,
}

/// A named filter view that can be invoked with @name syntax
#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// Include issues with this label
    pub label: Option<String>,
    /// Exclude issues with this label
    pub label_not: Option<String>,
    /// Include issues with any of these labels
    pub label_any: Option<Vec<String>>,
    /// Filter by state (open, closed)
    pub state: Option<String>,
    /// Show only issues assigned to current user
    #[serde(default)]
    pub mine: bool,
    /// Show only unassigned issues
    #[serde(default)]
    pub unassigned: bool,
    /// Filter by goal/milestone
    pub goal: Option<String>,
    /// Filter by exact priority
    pub priority: Option<u8>,
    /// Filter by priority <= value (0=urgent, 1=high, etc.)
    pub priority_lte: Option<u8>,
    /// Filter by priority >= value
    pub priority_gte: Option<u8>,
    /// Filter issues not updated in this duration (e.g., "30 days")
    pub updated_before: Option<String>,
    /// Filter issues updated within this duration
    pub updated_after: Option<String>,
    /// Filter issues created before this date/duration
    pub created_before: Option<String>,
    /// Filter issues created after this date/duration
    pub created_after: Option<String>,
    /// Sort order (priority, newest, oldest, updated)
    pub sort: Option<String>,
}

impl View {
    /// Convert view to a human-readable filter string for display
    pub fn to_filter_string(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref label) = self.label {
            parts.push(format!("--label={}", label));
        }
        if let Some(ref label) = self.label_not {
            parts.push(format!("--label-not={}", label));
        }
        if let Some(ref labels) = self.label_any {
            parts.push(format!("--label-any={}", labels.join(",")));
        }
        if let Some(ref state) = self.state {
            parts.push(format!("--state={}", state));
        }
        if self.mine {
            parts.push("--mine".to_string());
        }
        if self.unassigned {
            parts.push("--unassigned".to_string());
        }
        if let Some(ref goal) = self.goal {
            parts.push(format!("--goal={}", goal));
        }
        if let Some(p) = self.priority {
            parts.push(format!("--priority={}", p));
        }
        if let Some(p) = self.priority_lte {
            parts.push(format!("--priority-lte={}", p));
        }
        if let Some(p) = self.priority_gte {
            parts.push(format!("--priority-gte={}", p));
        }
        if let Some(ref d) = self.updated_before {
            parts.push(format!("--updated-before=\"{}\"", d));
        }
        if let Some(ref d) = self.updated_after {
            parts.push(format!("--updated-after=\"{}\"", d));
        }
        if let Some(ref d) = self.created_before {
            parts.push(format!("--created-before=\"{}\"", d));
        }
        if let Some(ref d) = self.created_after {
            parts.push(format!("--created-after=\"{}\"", d));
        }
        if let Some(ref sort) = self.sort {
            parts.push(format!("--sort={}", sort));
        }

        if parts.is_empty() {
            "(no filters)".to_string()
        } else {
            parts.join(" ")
        }
    }

    /// Check if the view has any filters defined
    pub fn is_empty(&self) -> bool {
        self.label.is_none()
            && self.label_not.is_none()
            && self.label_any.is_none()
            && self.state.is_none()
            && !self.mine
            && !self.unassigned
            && self.goal.is_none()
            && self.priority.is_none()
            && self.priority_lte.is_none()
            && self.priority_gte.is_none()
            && self.updated_before.is_none()
            && self.updated_after.is_none()
            && self.created_before.is_none()
            && self.created_after.is_none()
            && self.sort.is_none()
    }
}

/// Get the user config directory path
pub fn config_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(dirs.config_dir().to_path_buf())
}

/// Get the user config file path
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Load global user configuration
/// Returns default config if file doesn't exist
pub fn load() -> Result<UserConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(UserConfig::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let config: UserConfig = toml::from_str(&content)?;
    Ok(config)
}

/// Save global user configuration
pub fn save(config: &UserConfig) -> Result<()> {
    let path = config_path()?;

    // Create config directory if it doesn't exist
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Resolve json output setting - CLI flag overrides config default
pub fn resolve_json_default(cli_json: bool) -> Result<bool> {
    Ok(cli_json || load()?.defaults.json)
}

/// Resolve quiet mode setting - CLI flag overrides config default
pub fn resolve_quiet_default(cli_quiet: bool) -> Result<bool> {
    Ok(cli_quiet || load()?.defaults.quiet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config() {
        let config: UserConfig = toml::from_str("").unwrap();
        assert!(!config.defaults.json);
        assert!(!config.defaults.quiet);
        assert!(config.views.is_empty());
    }

    #[test]
    fn test_parse_defaults() {
        let toml = r#"
[defaults]
json = true
quiet = true
sort = "newest"
state = "open"
"#;
        let config: UserConfig = toml::from_str(toml).unwrap();
        assert!(config.defaults.json);
        assert!(config.defaults.quiet);
        assert_eq!(config.defaults.sort, Some("newest".to_string()));
        assert_eq!(config.defaults.state, Some("open".to_string()));
    }

    #[test]
    fn test_parse_view() {
        let toml = r#"
[views.bugs]
label = "bug"
state = "open"
mine = true
"#;
        let config: UserConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.views.len(), 1);

        let view = config.views.get("bugs").unwrap();
        assert_eq!(view.label, Some("bug".to_string()));
        assert_eq!(view.state, Some("open".to_string()));
        assert!(view.mine);
    }

    #[test]
    fn test_parse_view_with_priority() {
        let toml = r#"
[views.urgent]
priority_lte = 1
state = "open"
label_not = "wontfix"
"#;
        let config: UserConfig = toml::from_str(toml).unwrap();
        let view = config.views.get("urgent").unwrap();
        assert_eq!(view.priority_lte, Some(1));
        assert_eq!(view.label_not, Some("wontfix".to_string()));
    }

    #[test]
    fn test_parse_view_with_date_filters() {
        let toml = r#"
[views.stale]
state = "open"
updated_before = "30 days"
"#;
        let config: UserConfig = toml::from_str(toml).unwrap();
        let view = config.views.get("stale").unwrap();
        assert_eq!(view.updated_before, Some("30 days".to_string()));
    }

    #[test]
    fn test_parse_multiple_views() {
        let toml = r#"
[views.bugs]
label = "bug"

[views.features]
label = "enhancement"

[views.mine]
mine = true
"#;
        let config: UserConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.views.len(), 3);
        assert!(config.views.contains_key("bugs"));
        assert!(config.views.contains_key("features"));
        assert!(config.views.contains_key("mine"));
    }

    #[test]
    fn test_view_to_filter_string() {
        let view = View {
            label: Some("bug".to_string()),
            state: Some("open".to_string()),
            mine: true,
            ..Default::default()
        };
        let s = view.to_filter_string();
        assert!(s.contains("--label=bug"));
        assert!(s.contains("--state=open"));
        assert!(s.contains("--mine"));
    }

    #[test]
    fn test_view_to_filter_string_empty() {
        let view = View::default();
        assert_eq!(view.to_filter_string(), "(no filters)");
    }

    #[test]
    fn test_view_is_empty() {
        assert!(View::default().is_empty());

        let view = View {
            label: Some("bug".to_string()),
            ..Default::default()
        };
        assert!(!view.is_empty());
    }

    #[test]
    fn test_config_roundtrip() {
        let mut config = UserConfig::default();
        config.defaults.json = true;
        config.views.insert(
            "bugs".to_string(),
            View {
                label: Some("bug".to_string()),
                state: Some("open".to_string()),
                ..Default::default()
            },
        );

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: UserConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.defaults.json, config.defaults.json);
        assert_eq!(parsed.views.len(), 1);
        assert_eq!(parsed.views.get("bugs"), config.views.get("bugs"));
    }
}
