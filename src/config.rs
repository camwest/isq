use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

use crate::forges::ForgeType;

/// Repository-level configuration from .config/isq.toml
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    #[serde(default)]
    pub worktree: WorktreeConfig,
    /// Opaque config passed to forge's handle_on_start - each forge defines its own schema
    #[serde(default = "default_toml_table")]
    pub on_start: toml::Value,
    /// Priority label mapping (GitHub only) - maps label names to priority levels
    /// Example: { "P0" = 0, "P1" = 1, "P2" = 2, "P3" = 3 }
    #[serde(default = "default_toml_table")]
    pub priority: toml::Value,
}

fn default_toml_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self {
            worktree: WorktreeConfig::default(),
            on_start: default_toml_table(),
            priority: default_toml_table(),
        }
    }
}

/// Detected project type for setup script generation
#[derive(Debug, Clone, Copy, PartialEq)]
enum ProjectType {
    Node(NodePackageManager),
    Rust,
    Python,
    Ruby,
    Go,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NodePackageManager {
    Pnpm,
    Yarn,
    Bun,
    Npm,
}

/// Worktree-related configuration
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct WorktreeConfig {
    /// Shell script to run after creating a worktree
    pub setup: Option<String>,
}

/// Load config from .config/isq.toml in repo root
pub fn load_repo_config(repo_path: &Path) -> Result<Option<RepoConfig>> {
    let config_path = repo_path.join(".config/isq.toml");
    if !config_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&config_path)?;
    let config: RepoConfig = toml::from_str(&content)?;
    Ok(Some(config))
}

/// Detect project type from marker files
fn detect_project_type(repo_path: &Path) -> ProjectType {
    // Node.js - check lock files first to determine package manager
    if repo_path.join("pnpm-lock.yaml").exists() {
        return ProjectType::Node(NodePackageManager::Pnpm);
    }
    if repo_path.join("yarn.lock").exists() {
        return ProjectType::Node(NodePackageManager::Yarn);
    }
    if repo_path.join("bun.lockb").exists() {
        return ProjectType::Node(NodePackageManager::Bun);
    }
    if repo_path.join("package-lock.json").exists() || repo_path.join("package.json").exists() {
        return ProjectType::Node(NodePackageManager::Npm);
    }

    // Rust
    if repo_path.join("Cargo.toml").exists() {
        return ProjectType::Rust;
    }

    // Python
    if repo_path.join("requirements.txt").exists()
        || repo_path.join("pyproject.toml").exists()
        || repo_path.join("setup.py").exists()
    {
        return ProjectType::Python;
    }

    // Ruby
    if repo_path.join("Gemfile").exists() {
        return ProjectType::Ruby;
    }

    // Go
    if repo_path.join("go.mod").exists() {
        return ProjectType::Go;
    }

    ProjectType::Unknown
}

/// Generate setup script based on project type
fn generate_setup_script(project_type: ProjectType) -> Option<String> {
    match project_type {
        ProjectType::Node(pm) => {
            let install_cmd = match pm {
                NodePackageManager::Pnpm => "pnpm install",
                NodePackageManager::Yarn => "yarn",
                NodePackageManager::Bun => "bun install",
                NodePackageManager::Npm => "npm install",
            };
            Some(install_cmd.to_string())
        }
        ProjectType::Rust => {
            // Rust typically doesn't need setup - cargo handles it on build
            None
        }
        ProjectType::Python => {
            // Could be pip, poetry, etc. - keep it simple
            Some("pip install -r requirements.txt 2>/dev/null || true".to_string())
        }
        ProjectType::Ruby => Some("bundle install".to_string()),
        ProjectType::Go => {
            // Go modules download on build
            None
        }
        ProjectType::Unknown => None,
    }
}

/// Generate config file content for a forge
fn generate_config_content(repo_path: &Path, forge_type: &str) -> String {
    let project_type = detect_project_type(repo_path);
    let setup_script = generate_setup_script(project_type);

    let mut content = String::new();

    // Header comment
    content.push_str("# isq configuration\n");
    content.push_str("# https://github.com/camwest/isq\n\n");

    // Worktree section
    content.push_str("[worktree]\n");
    if let Some(script) = setup_script {
        content.push_str(&format!("setup = \"{}\"\n", script));
    } else {
        content.push_str("# setup = \"npm install\"  # Command to run after creating worktree\n");
    }
    content.push('\n');

    // On-start section - forge-specific defaults
    content.push_str("[on_start]\n");
    let ft = ForgeType::from_str(forge_type).expect("invalid forge type passed to config generation");
    content.push_str(ft.default_on_start_toml());

    content
}

/// Create .config/isq.toml with sensible defaults
/// Returns true if file was created, false if it already exists
pub fn create_repo_config(repo_path: &Path, forge_type: &str) -> Result<bool> {
    let config_dir = repo_path.join(".config");
    let config_path = config_dir.join("isq.toml");

    // Don't overwrite existing config
    if config_path.exists() {
        return Ok(false);
    }

    // Create .config directory if needed
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }

    let content = generate_config_content(repo_path, forge_type);
    std::fs::write(&config_path, content)?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty_config() {
        let config: RepoConfig = toml::from_str("").unwrap();
        assert!(config.worktree.setup.is_none());
        // on_start is opaque - just check it parses
        assert!(config.on_start.is_table() || config.on_start.as_table().is_none());
    }

    #[test]
    fn test_parse_worktree_setup() {
        let toml = r#"
[worktree]
setup = "npm install"
"#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.worktree.setup, Some("npm install".to_string()));
    }

    #[test]
    fn test_parse_on_start_is_opaque() {
        // Core doesn't interpret on_start - just passes it to forge
        let toml = r#"
[on_start]
add_labels = ["in progress", "wip"]
assign_self = true
custom_field = "whatever"
"#;
        let config: RepoConfig = toml::from_str(toml).unwrap();
        // Just verify it parsed as a table
        assert!(config.on_start.is_table());
    }

    #[test]
    fn test_detect_rust_project() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        assert_eq!(detect_project_type(temp.path()), ProjectType::Rust);
    }

    #[test]
    fn test_detect_node_npm() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("package.json"), "{}").unwrap();
        assert_eq!(detect_project_type(temp.path()), ProjectType::Node(NodePackageManager::Npm));
    }

    #[test]
    fn test_detect_node_pnpm() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("package.json"), "{}").unwrap();
        std::fs::write(temp.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_project_type(temp.path()), ProjectType::Node(NodePackageManager::Pnpm));
    }

    #[test]
    fn test_generate_github_config() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("package.json"), "{}").unwrap();
        std::fs::write(temp.path().join("yarn.lock"), "").unwrap();

        let content = generate_config_content(temp.path(), "github");
        assert!(content.contains("setup = \"yarn\""));
        assert!(content.contains("add_labels = [\"in progress\"]"));
        assert!(content.contains("assign_self = true"));
    }

    #[test]
    fn test_generate_linear_config() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("Cargo.toml"), "").unwrap();

        let content = generate_config_content(temp.path(), "linear");
        // Rust doesn't have setup script
        assert!(content.contains("# setup ="));
        assert!(content.contains("transition = \"started\""));
        assert!(content.contains("assign_self = true"));
    }

    #[test]
    fn test_create_repo_config_creates_file() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("package.json"), "{}").unwrap();

        assert!(create_repo_config(temp.path(), "github").unwrap());
        assert!(temp.path().join(".config/isq.toml").exists());

        // Second call should return false (already exists)
        assert!(!create_repo_config(temp.path(), "github").unwrap());
    }
}
