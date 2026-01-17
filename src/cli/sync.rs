//! Sync CLI command

use std::time::Instant;

use anyhow::Result;

use crate::config;
use crate::db;
use crate::forges::get_forge_for_repo;
use crate::repo;
use crate::user_config;

pub async fn cmd_sync(cli_quiet: bool) -> Result<()> {
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = user_config::resolve_quiet_default(cli_quiet)?;

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    if !quiet {
        eprintln!("Syncing {}...", link.forge_repo);
    }
    let start = Instant::now();

    let issues_result = forge.list_issues(&repo_struct).await?;
    let comments_result = forge.list_all_comments(&repo_struct).await?;
    let goals = forge.list_goals(&repo_struct).await?;
    let fetch_time = start.elapsed();

    // Extract items from FetchResult (PRs already filtered by forge)
    let mut issues = issues_result.items;
    let comments = comments_result.items;

    // Apply priority from repo config (each forge handles its own logic)
    if let Ok(Some(config)) = config::load_repo_config(std::path::Path::new(&repo_path)) {
        forge.apply_priority_config(&mut issues, &config.priority);
    }

    let conn = db::open()?;
    // isq sync is explicit full sync - use is_complete from fetch results
    db::save_issues(
        &conn,
        &link.forge_repo,
        &issues,
        true,
        issues_result.is_complete,
    )?;
    db::save_comments(
        &conn,
        &link.forge_repo,
        &comments,
        true,
        comments_result.is_complete,
    )?;
    db::save_goals(&conn, &link.forge_repo, &goals)?;

    // Touch repo to update last_accessed
    db::touch_repo(&conn, &repo_path)?;

    if !quiet {
        println!(
            "✓ Synced {} issues, {} comments, and {} goals in {:.2}s",
            issues.len(),
            comments.len(),
            goals.len(),
            fetch_time.as_secs_f64()
        );
    }

    Ok(())
}
