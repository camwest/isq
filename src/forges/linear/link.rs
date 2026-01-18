//! Linear link flow implementation

use anyhow::{Result, anyhow};

use super::client::LinearClient;
use super::oauth::oauth_flow;
use super::{AUTH, ForgeType};
use crate::forges::LinkArgs;
use crate::forges::LinkResult;
use crate::{config, db, repo};

/// Run the complete Linear link flow.
/// Handles auth, team selection, syncs issues, and returns the result.
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::Linear;
    let conn = db::open()?;

    // Try existing auth first, fall back to OAuth
    let (token, is_new_auth) = match AUTH.get_token() {
        Ok(t) => (t, false),
        Err(_) => {
            let oauth_token = oauth_flow().await?;
            let expires_at = oauth_token.expires_in.map(|secs| {
                (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339()
            });
            AUTH.store_credential(
                &oauth_token.access_token,
                oauth_token.refresh_token.as_deref(),
                expires_at.as_deref(),
            )?;
            (oauth_token.access_token, true)
        }
    };

    let client = LinearClient::new(token);

    // Verify authentication - get user ID for assignment and display name for printing
    let (user_id, user_display_name) = client.get_viewer().await?;
    if is_new_auth {
        println!("✓ Authenticated as {}", user_display_name);
    }

    // List teams
    let teams = client.list_teams().await?;
    if teams.is_empty() {
        anyhow::bail!("No teams found in your Linear workspace");
    }

    // Handle -o list-teams flag
    if args.has_flag("list-teams") {
        println!("Available teams:");
        for team in &teams {
            println!("  {} ({})", team.name, team.key);
        }
        // Return empty result for list-teams (caller should not save)
        return Err(anyhow!("-o list-teams: showing available teams"));
    }

    // Resolve team from -o team=X argument or auto-select if only one
    let team = if let Some(team_query) = args.get("team") {
        let query_lower = team_query.to_lowercase();
        teams
            .iter()
            .find(|t| t.name.to_lowercase() == query_lower || t.key.to_lowercase() == query_lower)
            .ok_or_else(|| {
                let available: Vec<_> = teams
                    .iter()
                    .map(|t| format!("{} ({})", t.name, t.key))
                    .collect();
                anyhow!(
                    "Team '{}' not found.\n\nAvailable teams:\n  {}",
                    team_query,
                    available.join("\n  ")
                )
            })?
    } else if teams.len() == 1 {
        println!("Using team: {} ({})", teams[0].name, teams[0].key);
        &teams[0]
    } else {
        let available: Vec<_> = teams
            .iter()
            .map(|t| format!("{} ({})", t.name, t.key))
            .collect();
        anyhow::bail!(
            "Multiple teams available. Specify one with -o team=<name>.\n\nAvailable teams:\n  {}\n\nExample: isq link linear -o team=\"{}\"",
            available.join("\n  "),
            teams[0].name
        );
    };

    // Get organization info for display name
    let org = client.get_organization().await?;
    let display_name = format!("{}/{}", org.url_key, team.key);
    let forge_repo = format!("{}/{}", team.key, team.id);

    // Create pseudo-repo for syncing (unused but kept for future reference)
    let _pseudo_repo = repo::Repo {
        owner: team.key.clone(),
        name: team.id.clone(),
    };

    // Sync issues
    println!("Syncing {}...", team.name);
    let issues_result = client.list_team_issues_internal(&team.id, None).await?;

    // Save to database (user_id for API calls, user_display_name for --mine filter)
    db::set_repo_link(
        &conn,
        repo_path,
        forge_type.as_str(),
        &forge_repo,
        Some(&display_name),
        Some(&user_id),
        Some(&user_display_name),
    )?;
    db::save_issues(
        &conn,
        &forge_repo,
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

    Ok(LinkResult {
        display_name: team.name.clone(),
    })
}
