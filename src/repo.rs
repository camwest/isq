use anyhow::{anyhow, Result};
use std::path::PathBuf;
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

/// Get the absolute path to the git repository root (working directory)
pub fn detect_repo_path() -> Result<String> {
    let repo = discover_repo()?;
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow!("Bare repository has no working directory"))?;
    // workdir() may return relative path, canonicalize to absolute
    let canonical = workdir
        .canonicalize()
        .map_err(|e| anyhow!("Failed to resolve workdir path: {}", e))?;
    Ok(canonical.to_string_lossy().to_string())
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
