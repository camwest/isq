use anyhow::{anyhow, Result};
use std::path::PathBuf;

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
