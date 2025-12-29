use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository identifier (owner/name)
#[derive(Debug, Clone)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Discover the git repository from current directory
fn discover_repo() -> Result<gix::Repository> {
    gix::discover(".").map_err(|e| anyhow!("Not a git repository: {}", e))
}

/// Get the git directory path (stable worktree identity)
///
/// For the main worktree, returns `/path/to/repo/.git`
/// For linked worktrees, returns `/path/to/repo/.git/worktrees/<name>`
///
/// This path is stable even if the worktree directory is moved.
pub fn detect_git_dir() -> Result<PathBuf> {
    let repo = discover_repo()?;
    let git_dir = repo.git_dir();
    // git_dir() may return relative path, canonicalize to absolute
    let canonical = git_dir
        .canonicalize()
        .map_err(|e| anyhow!("Failed to resolve git dir path: {}", e))?;
    Ok(canonical)
}

/// Get the absolute path to the main git repository root (working directory)
///
/// For worktrees, this returns the main repository path (not the worktree path).
/// This ensures that repo links work correctly from any worktree.
pub fn detect_repo_path() -> Result<String> {
    let repo = discover_repo()?;

    // Use common_dir to get the main repo's .git directory
    // For main repo: common_dir == git_dir == /path/to/repo/.git
    // For worktree: common_dir == /path/to/main/repo/.git (with /../.. that needs resolving)
    let common_dir = repo.common_dir();

    // Canonicalize first to resolve any /../.. components
    let canonical_git_dir = common_dir
        .canonicalize()
        .map_err(|e| anyhow!("Failed to resolve common git dir: {}", e))?;

    // Get parent of .git to find the main repo working directory
    let main_repo_path = canonical_git_dir
        .parent()
        .ok_or_else(|| anyhow!("Could not find parent of git directory"))?;

    Ok(main_repo_path.to_string_lossy().to_string())
}

/// Detect repository from git remote
pub fn detect_repo() -> Result<Repo> {
    let repo = discover_repo()?;

    // Get the "origin" remote URL
    let remote = repo
        .find_remote("origin")
        .map_err(|_| anyhow!("No 'origin' remote found"))?;

    let url = remote
        .url(gix::remote::Direction::Fetch)
        .ok_or_else(|| anyhow!("No fetch URL for 'origin' remote"))?;

    parse_repo_url(url.to_bstring().to_string().as_str())
}

/// Parse owner/name from various git URL formats
fn parse_repo_url(url: &str) -> Result<Repo> {
    // SSH: git@github.com:owner/repo.git
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        return parse_owner_name(rest);
    }

    // HTTPS: https://github.com/owner/repo.git
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return parse_owner_name(rest);
    }

    // GitLab SSH
    if let Some(rest) = url.strip_prefix("git@gitlab.com:") {
        return parse_owner_name(rest);
    }

    // GitLab HTTPS
    if let Some(rest) = url.strip_prefix("https://gitlab.com/") {
        return parse_owner_name(rest);
    }

    Err(anyhow!("Unsupported git remote URL format: {}", url))
}

fn parse_owner_name(path: &str) -> Result<Repo> {
    let path = path.trim_end_matches(".git");
    let parts: Vec<&str> = path.split('/').collect();

    if parts.len() < 2 {
        return Err(anyhow!("Could not parse owner/repo from: {}", path));
    }

    Ok(Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    })
}

/// Get the current branch name (if on a branch)
///
/// Returns None if HEAD is detached (not on a branch)
pub fn detect_current_branch() -> Result<Option<String>> {
    let repo = discover_repo()?;
    let head = repo.head().map_err(|e| anyhow!("Failed to read HEAD: {}", e))?;
    Ok(head.referent_name().map(|n| n.shorten().to_string()))
}

/// Slugify a string for use in branch names
///
/// Converts to lowercase, replaces non-alphanumeric chars with dashes,
/// collapses multiple dashes, and limits length.
pub fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(50)
        .collect()
}

/// Create a new worktree with a branch
///
/// Returns the path to the new worktree.
/// Worktree is created as a sibling to the main repo: ~/src/myapp -> ~/src/myapp-{branch}
pub fn create_worktree(branch: &str) -> Result<PathBuf> {
    let repo = discover_repo()?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow!("Bare repository has no working directory"))?;

    // Canonicalize to get absolute path
    let workdir = workdir
        .canonicalize()
        .map_err(|e| anyhow!("Failed to resolve workdir path: {}", e))?;

    // Worktree location: sibling to main repo
    let parent = workdir
        .parent()
        .ok_or_else(|| anyhow!("Cannot determine parent directory"))?;
    let repo_name = workdir
        .file_name()
        .ok_or_else(|| anyhow!("Cannot determine repo name"))?
        .to_string_lossy();

    let worktree_path = parent.join(format!("{}-{}", repo_name, branch));

    // Single command: create worktree AND branch
    let output = Command::new("git")
        .arg("-C")
        .arg(&workdir)
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(branch)
        .arg(&worktree_path)
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(worktree_path)
}

/// Remove a worktree
///
/// Uses --force to handle uncommitted changes.
pub fn remove_worktree(worktree_path: &std::path::Path) -> Result<()> {
    let output = Command::new("git")
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path)
        .output()?;

    if !output.status.success() {
        return Err(anyhow!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

/// Run setup script in worktree directory
///
/// Environment variables available to the script:
/// - `ISQ_MAIN_WORKTREE`: Path to the main worktree
/// - `ISQ_ISSUE_NUMBER`: The issue number being worked on
/// - `ISQ_WORKTREE_PATH`: Path to the new worktree
pub async fn run_setup_script(
    worktree_path: &std::path::Path,
    script: &str,
    main_worktree: &std::path::Path,
    issue_number: u64,
) -> Result<()> {
    use tokio::process::Command as TokioCommand;

    let output = TokioCommand::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(worktree_path)
        .env("ISQ_MAIN_WORKTREE", main_worktree)
        .env("ISQ_ISSUE_NUMBER", issue_number.to_string())
        .env("ISQ_WORKTREE_PATH", worktree_path)
        .output()
        .await?;

    if !output.status.success() {
        return Err(anyhow!(
            "Setup script failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

// --- Git Hooks ---

const HOOK_MARKER: &str = "# Installed by isq";

const HOOK_SCRIPT: &str = r##"#!/bin/sh
# Installed by isq - remove with: isq unlink

ISSUE=$(isq current --quiet 2>/dev/null)

if [ -n "$ISSUE" ] && ! grep -q "\[#$ISSUE\]" "$1"; then
    sed -i.bak "1s/$/ [#$ISSUE]/" "$1" && rm -f "$1.bak"
fi
"##;

/// Install prepare-commit-msg hook
///
/// Returns Ok(true) if hook was installed, Ok(false) if already installed.
/// Errors if a non-isq hook already exists.
pub fn install_hook(repo_path: &Path) -> Result<bool> {
    let hook_path = repo_path.join(".git/hooks/prepare-commit-msg");

    if hook_path.exists() {
        let content = fs::read_to_string(&hook_path)?;
        if content.contains(HOOK_MARKER) {
            return Ok(false); // Already installed
        }
        anyhow::bail!("prepare-commit-msg hook already exists (not from isq)");
    }

    // Ensure hooks directory exists
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&hook_path, HOOK_SCRIPT)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(true)
}

/// Uninstall prepare-commit-msg hook (only if ours)
///
/// Returns Ok(true) if hook was removed, Ok(false) if not present or not ours.
pub fn uninstall_hook(repo_path: &Path) -> Result<bool> {
    let hook_path = repo_path.join(".git/hooks/prepare-commit-msg");

    if !hook_path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(&hook_path)?;
    if !content.contains(HOOK_MARKER) {
        return Ok(false); // Not ours, leave it alone
    }

    fs::remove_file(&hook_path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_git_dir() {
        // This test runs from within the isq repo
        let git_dir = detect_git_dir().unwrap();
        assert!(
            git_dir.ends_with(".git")
                || git_dir.to_string_lossy().contains(".git/worktrees/")
        );
    }

    #[test]
    fn test_detect_repo_path() {
        let path = detect_repo_path().unwrap();
        assert!(path.contains("isq"));
    }

    #[test]
    fn test_parse_repo_url_github_ssh() {
        let repo = parse_repo_url("git@github.com:owner/repo.git").unwrap();
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn test_parse_repo_url_github_https() {
        let repo = parse_repo_url("https://github.com/owner/repo.git").unwrap();
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
    }
}
