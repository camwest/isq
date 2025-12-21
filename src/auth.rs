use anyhow::{anyhow, Result};
use std::process::Command;

// TODO: Abstract into ForgeAuth trait for multi-backend support
// - GitHubAuth: gh CLI token detection
// - GitLabAuth: glab CLI or GITLAB_TOKEN env
// - ForgejoAuth: tea CLI or FORGEJO_TOKEN env
// - LinearAuth: linear CLI (github.com/schpet/linear-cli) or LINEAR_API_KEY env

/// Get GitHub token from gh CLI
pub fn get_gh_token() -> Result<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .map_err(|_| anyhow!("gh CLI not found. Install it from https://cli.github.com"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh CLI not authenticated. Run `gh auth login` first.\n{}",
            stderr.trim()
        ));
    }

    let token = String::from_utf8(output.stdout)?.trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("gh CLI returned empty token"));
    }

    Ok(token)
}
