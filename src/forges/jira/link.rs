//! JIRA link flow implementation

use anyhow::{Result, anyhow};

use super::client::JiraClient;
use super::oauth::{
    JiraAuthMode, JiraCredentials, get_accessible_resources, get_credentials_from_env,
    get_stored_credentials, oauth_flow, store_credentials,
};
use crate::forges::{ForgeType, LinkArgs, LinkResult};
use crate::{config, db, repo};

/// Run the complete JIRA link flow.
/// Handles auth, site selection, project selection, syncs issues, and returns the result.
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::Jira;
    let conn = db::open()?;

    // Try auth in order: env var (for CI) -> keyring -> OAuth
    let creds = if let Ok(env_creds) = get_credentials_from_env() {
        println!("Using JIRA credentials from JIRA_API_TOKEN");
        env_creds
    } else if let Ok(stored_creds) = get_stored_credentials() {
        println!("Using existing JIRA credentials");
        stored_creds
    } else {
        // Run OAuth flow
        let token = oauth_flow().await?;

        // Get accessible sites
        let sites = get_accessible_resources(&token.access_token).await?;
        if sites.is_empty() {
            anyhow::bail!("No JIRA sites accessible with this account");
        }

        // Select site (auto if one, otherwise require -o site=X)
        let site = if sites.len() == 1 {
            println!("Using site: {}", sites[0].name);
            &sites[0]
        } else {
            // Check for site argument
            if let Some(site_name) = args.get("site") {
                sites
                    .iter()
                    .find(|s| {
                        s.name.to_lowercase() == site_name.to_lowercase()
                            || s.url.contains(site_name)
                    })
                    .ok_or_else(|| {
                        let available: Vec<_> = sites.iter().map(|s| &s.name).collect();
                        anyhow!(
                            "Site '{}' not found. Available sites: {:?}",
                            site_name,
                            available
                        )
                    })?
            } else {
                let available: Vec<_> = sites.iter().map(|s| &s.name).collect();
                anyhow::bail!(
                    "Multiple JIRA sites available. Specify one with -o site=<name>.\n\nAvailable sites: {:?}",
                    available
                );
            }
        };

        let expires_at = token
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs as i64);

        let creds = JiraCredentials {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            auth_mode: JiraAuthMode::OAuth {
                cloud_id: site.id.clone(),
            },
            site_url: site.url.clone(),
            expires_at,
        };

        // Store credentials
        store_credentials(&creds)?;

        creds
    };

    let client = JiraClient::new(creds.clone());

    // List available projects
    let projects = client.list_projects().await?;
    if projects.is_empty() {
        anyhow::bail!("No projects found in this JIRA site");
    }

    // Handle -o list-projects flag
    if args.has_flag("list-projects") {
        println!("Available projects:");
        for project in &projects {
            println!("  {} - {}", project.key, project.name);
        }
        return Err(anyhow!("-o list-projects: showing available projects"));
    }

    // Resolve project from -o project=X argument or auto-select if only one
    let project = if let Some(project_query) = args.get("project") {
        let query_lower = project_query.to_lowercase();
        projects
            .iter()
            .find(|p| p.key.to_lowercase() == query_lower || p.name.to_lowercase() == query_lower)
            .ok_or_else(|| {
                let available: Vec<_> = projects
                    .iter()
                    .map(|p| format!("{} ({})", p.key, p.name))
                    .collect();
                anyhow!(
                    "Project '{}' not found.\n\nAvailable projects:\n  {}",
                    project_query,
                    available.join("\n  ")
                )
            })?
    } else if projects.len() == 1 {
        println!("Using project: {} ({})", projects[0].key, projects[0].name);
        &projects[0]
    } else {
        let available: Vec<_> = projects
            .iter()
            .map(|p| format!("{} ({})", p.key, p.name))
            .collect();
        anyhow::bail!(
            "Multiple projects available. Specify one with -o project=<key>.\n\nAvailable projects:\n  {}\n\nExample: isq link jira -o project=\"{}\"",
            available.join("\n  "),
            projects[0].key
        );
    };

    // Check write permissions
    if !client.check_write_permission(&project.key).await? {
        anyhow::bail!(
            "You don't have write access to project {}. isq requires write permissions to function properly.",
            project.key
        );
    }

    // Get current user for display
    let user = client.get_current_user().await?;
    let display_name = user.display_name.unwrap_or_else(|| user.account_id.clone());

    // Create repo identifier: site/project_key
    let site_host = creds
        .site_url
        .replace("https://", "")
        .replace("http://", "");
    let forge_repo = format!("{}/{}", site_host, project.key);

    // Create pseudo-repo for syncing (JIRA uses site_host as owner, project_key as name)
    let pseudo_repo = repo::Repo {
        owner: site_host.clone(),
        name: project.key.clone(),
    };

    // Sync issues
    println!("Syncing issues from {}...", project.key);
    let issues = client.list_issues_internal(&pseudo_repo, None).await?;

    // Save to database
    let full_display_name = format!("{} ({})", project.name, display_name);
    db::set_repo_link(
        &conn,
        db::SetRepoLinkParams {
            repo_path,
            forge_type: forge_type.as_str(),
            forge_repo: &forge_repo,
            auth_scope: None,
            display_name: Some(&full_display_name),
            user_id: Some(&user.account_id),
            user_name: Some(&display_name),
        },
    )?;
    db::save_issues(&conn, &forge_repo, &issues.items, true, true)?;
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

    println!("✓ Synced {} issues", issues.items.len());

    // Sync goals
    let goals = client.list_goals(&pseudo_repo).await?;
    db::save_goals(&conn, &forge_repo, &goals)?;
    if !goals.is_empty() {
        println!("✓ Synced {} versions", goals.len());
    }

    Ok(LinkResult {
        display_name: full_display_name,
    })
}
