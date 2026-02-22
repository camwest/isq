//! Linear link flow implementation

use anyhow::{Result, anyhow};

use super::client::LinearClient;
use super::oauth::oauth_flow;
use super::types::LinearTeam;
use super::{AUTH, ForgeType, build_auth_scope, store_scoped_credential};
use crate::forges::LinkArgs;
use crate::forges::LinkResult;
use crate::{config, db, repo};

fn find_team<'a>(teams: &'a [LinearTeam], team_query: &str) -> Option<&'a LinearTeam> {
    let query_lower = team_query.to_lowercase();
    teams
        .iter()
        .find(|t| t.name.to_lowercase() == query_lower || t.key.to_lowercase() == query_lower)
}

fn team_not_found_error(team_query: &str, teams: &[LinearTeam]) -> anyhow::Error {
    let available: Vec<_> = teams
        .iter()
        .map(|t| format!("{} ({})", t.name, t.key))
        .collect();
    anyhow!(
        "Team '{}' not found.\n\nAvailable teams:\n  {}",
        team_query,
        available.join("\n  ")
    )
}

/// Run the complete Linear link flow.
/// Handles auth, team selection, syncs issues, and returns the result.
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::Linear;
    let conn = db::open()?;
    let force_reauth = args.has_flag("reauth");
    let mut token: String;
    let mut is_new_auth = false;
    let mut refresh_token: Option<String> = None;
    let mut expires_at: Option<String> = None;

    // Try existing auth first, fall back to OAuth.
    if force_reauth {
        let oauth_token = oauth_flow().await?;
        expires_at = oauth_token
            .expires_in
            .map(|secs| (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());
        refresh_token = oauth_token.refresh_token.clone();
        AUTH.store_credential(
            &oauth_token.access_token,
            refresh_token.as_deref(),
            expires_at.as_deref(),
        )?;
        token = oauth_token.access_token;
        is_new_auth = true;
    } else {
        token = match AUTH.get_token() {
            Ok(t) => t,
            Err(_) => {
                let oauth_token = oauth_flow().await?;
                expires_at = oauth_token.expires_in.map(|secs| {
                    (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339()
                });
                refresh_token = oauth_token.refresh_token.clone();
                AUTH.store_credential(
                    &oauth_token.access_token,
                    refresh_token.as_deref(),
                    expires_at.as_deref(),
                )?;
                is_new_auth = true;
                oauth_token.access_token
            }
        }
    }

    let mut client = LinearClient::new(token.clone());

    // Verify authentication - get user ID for assignment and display name for printing
    let mut viewer = client.get_viewer().await?;
    let mut teams = client.list_teams().await?;
    if teams.is_empty() {
        anyhow::bail!("No teams found in your Linear workspace");
    }

    // If requested team is not available, retry with OAuth once.
    if let Some(team_query) = args.get("team")
        && find_team(&teams, team_query).is_none()
        && !force_reauth
        && !is_new_auth
    {
        println!(
            "Team '{}' not found in current Linear auth context. Re-authenticating...",
            team_query
        );
        let oauth_token = oauth_flow().await?;
        expires_at = oauth_token
            .expires_in
            .map(|secs| (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());
        refresh_token = oauth_token.refresh_token.clone();
        AUTH.store_credential(
            &oauth_token.access_token,
            refresh_token.as_deref(),
            expires_at.as_deref(),
        )?;

        token = oauth_token.access_token;
        is_new_auth = true;
        client = LinearClient::new(token.clone());
        viewer = client.get_viewer().await?;
        teams = client.list_teams().await?;
        if teams.is_empty() {
            anyhow::bail!("No teams found in your Linear workspace");
        }
    }

    let (user_id, user_display_name) = viewer;
    if is_new_auth {
        println!("✓ Authenticated as {}", user_display_name);
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
        find_team(&teams, team_query).ok_or_else(|| team_not_found_error(team_query, &teams))?
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
    let auth_scope = build_auth_scope(&org.url_key, &user_id);

    // Persist scoped credentials for this repo/account/workspace.
    // Reuse global stored metadata when token came from existing auth.
    if refresh_token.is_none()
        && expires_at.is_none()
        && let Some(cred) = AUTH.get_credential()?
        && cred.access_token == token
    {
        refresh_token = cred.refresh_token;
        expires_at = cred.expires_at;
    }
    store_scoped_credential(
        &auth_scope,
        &token,
        refresh_token.as_deref(),
        expires_at.as_deref(),
    )?;

    // Use scoped client so refreshes write back to this repo scope.
    client = LinearClient::new_with_scope(token.clone(), Some(auth_scope.clone()));

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
        Some(&auth_scope),
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
