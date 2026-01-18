//! Worktree-related CLI commands (current, start, cleanup)

use std::time::Instant;

use anyhow::Result;

use crate::config;
use crate::db;
use crate::display;
use crate::forges::{get_forge_for_repo, not_linked_error};
use crate::repo;

use super::utils::is_offline_error;

pub fn cmd_current(quiet: bool) -> Result<()> {
    let git_dir = repo::detect_git_dir()?;
    let conn = db::open()?;

    match db::get_worktree_issue(&conn, &git_dir.to_string_lossy())? {
        Some((_, issue_number)) => {
            println!("{}", issue_number);
            Ok(())
        }
        None => {
            if !quiet {
                eprintln!("No current issue. Use `isq start <number>` to set one.");
            }
            std::process::exit(1);
        }
    }
}

pub fn cmd_home() -> Result<()> {
    let git_dir = repo::detect_git_dir()?;
    let conn = db::open()?;

    match db::get_worktree_issue(&conn, &git_dir.to_string_lossy())? {
        Some((forge_repo, issue_id)) => {
            let start = Instant::now();
            let issue = db::load_issue(&conn, &forge_repo, &issue_id)?;
            let comments = db::load_comments(&conn, &forge_repo, &issue_id)?;
            let elapsed = start.elapsed();

            match issue {
                Some(issue) => {
                    display::print_issue(&issue, &comments, elapsed.as_millis() as u64);

                    // Git context
                    if let Ok(Some(branch)) = repo::detect_current_branch() {
                        println!();
                        println!("Branch: {}", branch);
                    }
                    if let Ok(path) = std::env::current_dir() {
                        println!("Worktree: {}", path.display());
                    }
                }
                None => {
                    let issue_display = display::format_issue_id(&issue_id);
                    eprintln!(
                        "Current issue {} not in cache. Run `isq sync` to refresh.",
                        issue_display
                    );
                    std::process::exit(1);
                }
            }
        }
        None => {
            eprintln!("No current issue. Use `isq start <number>` to set one.");
            eprintln!("Tip: Run `isq issue list` to see available issues.");
            std::process::exit(1);
        }
    }

    Ok(())
}

pub async fn cmd_start(id: String) -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Get linked forge repo
    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    // Load issue from cache (fast!)
    let issue_display = display::format_issue_id(&id);
    let issue = db::load_issue(&conn, &link.forge_repo, &id)?.ok_or_else(|| {
        anyhow::anyhow!("Issue {} not found. Run `isq sync` first.", issue_display)
    })?;

    // Create branch name: {id}-{slugified-title}
    let branch = format!("{}-{}", id, repo::slugify(&issue.title));

    // Load and validate config BEFORE creating worktree (fail fast)
    let repo_config = config::load_repo_config(std::path::Path::new(&repo_path))?;

    if let Some(ref cfg) = repo_config {
        // Validate on_start config with forge
        let (forge, _) = crate::forges::get_forge_for_repo(&repo_path)?;
        forge.validate_on_start_config(&cfg.on_start)?;
    }

    // Create worktree (blocking, ~50-100ms)
    let worktree_path = repo::create_worktree(&branch)?;

    println!("Created worktree {}", worktree_path.display());
    println!("Branch: {}", branch);

    // Get git_dir for the NEW worktree (for DB association)
    let orig_dir = std::env::current_dir()?;
    std::env::set_current_dir(&worktree_path)?;
    let git_dir = repo::detect_git_dir()?;
    std::env::set_current_dir(orig_dir)?;

    // Clone values for async blocks
    let worktree_path_clone = worktree_path.clone();
    let repo_path_clone = repo_path.clone();
    let git_dir_str = git_dir.to_string_lossy().to_string();
    let forge_repo = link.forge_repo.clone();
    let user_id = link.user_id.clone();

    // Run DB association, setup script, and forge actions concurrently
    let id_for_db = id.clone();
    let id_for_setup = id.clone();
    let id_for_forge = id.clone();
    let db_future = async { db::set_worktree_issue(&conn, &git_dir_str, &forge_repo, &id_for_db) };

    let setup_future = async {
        if let Some(ref cfg) = repo_config {
            if let Some(ref script) = cfg.worktree.setup {
                let start = Instant::now();
                match repo::run_setup_script(
                    &worktree_path_clone,
                    script,
                    std::path::Path::new(&repo_path_clone),
                    &id_for_setup,
                )
                .await
                {
                    Ok(()) => {
                        println!(
                            "Running setup... done ({:.1}s)",
                            start.elapsed().as_secs_f32()
                        );
                    }
                    Err(e) => {
                        eprintln!("Setup warning: {}", e);
                    }
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    };

    let forge_future = async {
        if let Some(ref cfg) = repo_config {
            let on_start = &cfg.on_start;

            // Check if on_start has any config (non-empty table)
            let has_config = on_start.as_table().map(|t| !t.is_empty()).unwrap_or(false);

            if has_config {
                // Get forge client
                let (forge, _) = match get_forge_for_repo(&repo_path) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("Forge warning: {} (will sync later)", e);
                        return Ok::<_, anyhow::Error>(());
                    }
                };

                // Parse forge_repo for API calls
                let parts: Vec<&str> = forge_repo.split('/').collect();
                if parts.len() != 2 {
                    eprintln!("Forge warning: invalid forge_repo format");
                    return Ok(());
                }
                let repo_struct = repo::Repo {
                    owner: parts[0].to_string(),
                    name: parts[1].to_string(),
                };

                // Handle on_start - forge interprets config and handles everything
                // (labels, transitions, assign_self, etc. are all forge-specific)
                if let Err(e) = forge
                    .handle_on_start(&repo_struct, &id_for_forge, on_start, user_id.as_deref())
                    .await
                {
                    if !is_offline_error(&e) {
                        eprintln!("on_start warning: {}", e);
                    }
                }

                println!("Marked in progress");
            }
        }
        Ok::<_, anyhow::Error>(())
    };

    // Run all three concurrently
    let (db_result, setup_result, forge_result) =
        tokio::join!(db_future, setup_future, forge_future);

    // DB error is fatal
    db_result?;

    // Setup and forge errors are warnings (already printed)
    let _ = setup_result;
    let _ = forge_result;

    println!("Issue {}: \"{}\"", issue_display, issue.title);

    Ok(())
}

pub async fn cmd_cleanup(keep: bool) -> Result<()> {
    let git_dir = repo::detect_git_dir()?;
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if we have a current issue
    let (forge_repo, issue_id) = db::get_worktree_issue(&conn, &git_dir.to_string_lossy())?
        .ok_or_else(|| anyhow::anyhow!("No current issue. Nothing to clean up."))?;

    let worktree_path = std::env::current_dir()?;

    // Run forge cleanup actions BEFORE clearing DB (we need the association info)
    run_on_cleanup_hooks(&repo_path, &forge_repo, &issue_id).await;

    // Clear the DB association
    db::clear_worktree_issues(&conn, &git_dir.to_string_lossy())?;

    if keep {
        println!("Cleared issue #{} association", issue_id);
        println!("(worktree kept at {})", worktree_path.display());
    } else {
        // Remove the worktree
        repo::remove_worktree(&worktree_path)?;
        println!("Removed worktree {}", worktree_path.display());
        println!("Cleared issue #{} association", issue_id);
    }

    Ok(())
}

/// Run on_cleanup lifecycle hooks if configured
async fn run_on_cleanup_hooks(repo_path: &str, forge_repo: &str, issue_id: &str) {
    let repo_config = match config::load_repo_config(std::path::Path::new(repo_path)) {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return,
        Err(e) => {
            eprintln!("Config warning: {}", e);
            return;
        }
    };

    let on_cleanup = &repo_config.on_cleanup;

    // Skip if no cleanup config
    if !on_cleanup.as_table().is_some_and(|t| !t.is_empty()) {
        return;
    }

    // Get forge client
    let (forge, link) = match get_forge_for_repo(repo_path) {
        Ok(f) => f,
        Err(e) => {
            if !e.to_string().contains("not linked") {
                eprintln!("Forge warning: {} (skipping cleanup actions)", e);
            }
            return;
        }
    };

    // Parse forge_repo for API calls
    let parts: Vec<&str> = forge_repo.split('/').collect();
    if parts.len() != 2 {
        eprintln!("Warning: invalid forge_repo format");
        return;
    }

    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    // Handle on_cleanup - forge interprets config
    if let Err(e) = forge
        .handle_on_cleanup(&repo_struct, issue_id, on_cleanup, link.user_id.as_deref())
        .await
    {
        if !is_offline_error(&e) {
            eprintln!("on_cleanup warning: {}", e);
        }
    } else {
        println!("Cleaned up issue state");
    }
}
