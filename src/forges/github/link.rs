//! GitHub repository linking flow
//!
//! Handles authentication, verification, issue sync, and database setup.

use anyhow::Result;

use super::AUTH;
use super::client::GitHubClient;
use super::oauth::oauth_flow;
use crate::forges::{Forge, ForgeType, LinkArgs, LinkResult};
use crate::{config, db, repo};

/// Run the complete GitHub link flow.
/// Handles auth, verifies credentials, syncs issues, and returns the result.
pub async fn link(repo_path: &str, _args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::GitHub;
    let conn = db::open()?;

    // Detect GitHub repo from git remote
    let repo = repo::detect_repo()?;

    // Try existing auth first, fall back to OAuth
    let (token, auth_method) = match AUTH.get_token() {
        Ok(t) => {
            // Store in keychain so daemon can access
            // (gh CLI isn't available from launchd)
            AUTH.store_credential(&t, None, None)?;
            (t, "existing")
        }
        Err(_) => {
            let oauth_token = oauth_flow().await?;
            AUTH.store_credential(
                &oauth_token.access_token,
                oauth_token.refresh_token.as_deref(),
                None, // GitHub tokens don't expire by default
            )?;
            (oauth_token.access_token, "OAuth")
        }
    };

    let client = GitHubClient::new(token);

    // Verify authentication
    let username = client.get_user().await?;
    println!("✓ Authenticated as {} (via {})", username, auth_method);

    // Sync issues
    let display_name = repo.full_name();
    println!("Syncing {}...", display_name);
    let issues_result = client.list_issues(&repo).await?;

    // Save to database (for GitHub, username serves as both user_id and user_name)
    db::set_repo_link(
        &conn,
        repo_path,
        forge_type.as_str(),
        &repo.full_name(),
        None,
        Some(&display_name),
        Some(&username),
        Some(&username),
    )?;
    db::save_issues(
        &conn,
        &repo.full_name(),
        &issues_result.items,
        true,
        issues_result.is_complete,
    )?;
    db::add_watched_repo(&conn, repo_path)?;

    // Create .config/isq.toml with defaults
    if config::create_repo_config(std::path::Path::new(repo_path), forge_type.as_str())? {
        println!("✓ Created .config/isq.toml");
    }

    // Install commit hook
    match repo::install_hook(std::path::Path::new(repo_path)) {
        Ok(true) => println!("✓ Installed commit hook"),
        Ok(false) => {} // Already installed, silent
        Err(e) => eprintln!("Warning: Could not install hook: {}", e),
    }

    println!("✓ Cached {} issues", issues_result.items.len());

    Ok(LinkResult { display_name })
}
