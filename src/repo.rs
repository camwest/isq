use anyhow::{anyhow, Result};
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

/// Detect repository from git remote
pub fn detect_repo() -> Result<Repo> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|_| anyhow!("git not found"))?;

    if !output.status.success() {
        return Err(anyhow!("Not a git repository or no 'origin' remote"));
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    parse_repo_url(&url)
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
