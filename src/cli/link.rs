//! Link, unlink, and logout CLI commands

use anyhow::Result;

use crate::credentials;
use crate::db;
use crate::forges::{ALL_FORGE_TYPES, ForgeType, LinkArgs};
use crate::repo;
use crate::service;

use super::utils::ensure_service_running;

pub async fn cmd_link(forge_name: Option<&str>, opts: Vec<String>) -> Result<()> {
    let repo_path = repo::detect_repo_path()?;

    // Require forge name
    let forge_name = forge_name.ok_or_else(|| {
        let forges: Vec<_> = ALL_FORGE_TYPES
            .iter()
            .map(|f| format!("  isq link {}", f.as_str()))
            .collect();
        anyhow::anyhow!("Missing forge name.\n\nRun one of:\n{}", forges.join("\n"))
    })?;

    // Parse forge type
    let forge_type = ForgeType::from_str(forge_name).ok_or_else(|| {
        let forges: Vec<_> = ALL_FORGE_TYPES
            .iter()
            .map(|f| format!("  isq link {}", f.as_str()))
            .collect();
        anyhow::anyhow!(
            "Unknown forge: {}\n\nRun one of:\n{}",
            forge_name,
            forges.join("\n")
        )
    })?;

    // Parse options
    let args = LinkArgs::parse(&opts)?;

    // Run forge-specific link flow
    let result = forge_type.link(&repo_path, &args).await?;

    // Start background service
    println!();
    ensure_service_running()?;
    println!(
        "\n✓ Linked to {} ({})",
        forge_type.auth().display_name,
        result.display_name
    );

    Ok(())
}

pub fn cmd_unlink() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if linked
    let link = db::get_repo_link(&conn, &repo_path)?;
    if link.is_none() {
        println!("This repo is not linked to any issue tracker.");
        return Ok(());
    }

    let link = link.unwrap();

    // Remove commit hook (silently skip if not ours)
    if repo::uninstall_hook(std::path::Path::new(&repo_path))? {
        println!("✓ Removed commit hook");
    }

    db::remove_repo_link(&conn, &repo_path)?;
    db::remove_watched_repo(&conn, &repo_path)?;

    println!("✓ Unlinked from {} ({})", link.forge_type, link.forge_repo);

    // Check if any repos left - if not, uninstall service
    let remaining = db::list_watched_repos(&conn)?;
    if remaining.is_empty() {
        println!();
        service::uninstall()?;
        println!("✓ System service removed (no repos to watch)");
    }

    Ok(())
}

pub fn cmd_logout(forge_name: Option<&str>) -> Result<()> {
    // Require forge name with helpful error
    let forge_name = forge_name.ok_or_else(|| {
        let forges: Vec<_> = ALL_FORGE_TYPES
            .iter()
            .map(|f| format!("  isq logout {}", f.as_str()))
            .collect();
        anyhow::anyhow!("Missing forge name.\n\nRun one of:\n{}", forges.join("\n"))
    })?;

    let forge_type = ForgeType::from_str(forge_name).ok_or_else(|| {
        let forges: Vec<_> = ALL_FORGE_TYPES
            .iter()
            .map(|f| format!("  isq logout {}", f.as_str()))
            .collect();
        anyhow::anyhow!(
            "Unknown forge: {}\n\nRun one of:\n{}",
            forge_name,
            forges.join("\n")
        )
    })?;

    let auth = forge_type.auth();
    let has_scoped_linear = matches!(forge_type, ForgeType::Linear)
        && credentials::list_services()?
            .iter()
            .any(|service| service.starts_with("linear:"));

    // Check if credential exists first
    if !auth.has_credentials() && !has_scoped_linear {
        println!("No stored credentials for {}.", auth.display_name);
        return Ok(());
    }

    credentials::remove_credential(auth.keyring_service)?;
    if matches!(forge_type, ForgeType::Linear) {
        let removed = credentials::remove_credentials_with_prefix("linear:")?;
        if removed > 0 {
            println!("✓ Removed {} scoped Linear credential(s)", removed);
        }
    }
    println!("✓ Logged out from {}", auth.display_name);

    // Note about env vars if relevant
    if std::env::var(auth.env_var).is_ok() {
        println!("  Note: {} is still set in your environment", auth.env_var);
    }

    Ok(())
}
