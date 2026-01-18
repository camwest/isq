//! Daemon-related CLI commands

use anyhow::Result;

use crate::db;
use crate::forges::{ALL_FORGE_TYPES, not_linked_error};
use crate::repo;
use crate::service;

pub fn cmd_status() -> Result<()> {
    // Check service status
    let status = service::status()?;

    if !status.installed {
        println!("Service: not installed");
        println!("         Run `isq link <forge>` to install");
    } else if !status.running {
        println!("Service: installed but not running");
    } else if let Some(pid) = status.pid {
        println!("Service: running (PID {})", pid);
    } else {
        println!("Service: running");
    }

    // Clean up stale repo entries before displaying
    let conn = db::open()?;
    let removed = db::cleanup_stale_repos(&conn)?;
    if removed > 0 {
        println!("\n(Cleaned up {} stale entries)", removed);
    }

    // Show rate limit budget per forge
    let mut shown_rate_limits = false;
    for forge_type in ALL_FORGE_TYPES {
        if let Some(state) = db::get_rate_limit_state(&conn, forge_type.as_str())? {
            if let (Some(limit), Some(_remaining)) = (state.limit, state.remaining) {
                if !shown_rate_limits {
                    println!();
                    shown_rate_limits = true;
                }
                let used = state.used().unwrap_or(0);
                println!(
                    "Rate limit budget ({}): {} req/hr",
                    forge_type.auth().display_name,
                    limit
                );
                println!("  Used this hour: {}", used);
            }
        }
    }

    // Show all watched sources
    let watched = db::list_watched_repos(&conn)?;

    if watched.is_empty() {
        println!("\nNothing being watched.");
        println!("Run `isq link <forge>` in a git repo to add it.");
    } else {
        println!("\nWatching:");
        for watched_repo in &watched {
            // Look up the link to get forge info
            let link = db::get_repo_link(&conn, &watched_repo.repo)?;
            let (display, forge_repo, forge_type) = match &link {
                Some(l) => {
                    // Use display_name if available, fall back to forge_repo
                    let display = l
                        .display_name
                        .clone()
                        .unwrap_or_else(|| l.forge_repo.clone());
                    (display, l.forge_repo.clone(), l.forge_type.clone())
                }
                None => (
                    watched_repo.repo.clone(),
                    watched_repo.repo.clone(),
                    "unknown".to_string(),
                ),
            };

            let sync_state = db::get_sync_state(&conn, &forge_repo)?;
            let pending = db::count_pending_ops(&conn, &forge_repo)?;

            let sync_info = match sync_state {
                Some(s) => {
                    let last_sync = s.last_sync.as_deref().unwrap_or("never");
                    format!("{} issues ({})", s.issue_count, last_sync)
                }
                None => "not synced".to_string(),
            };

            let pending_info = if pending > 0 {
                format!(" [{} pending]", pending)
            } else {
                String::new()
            };

            // Check if this forge is rate limited
            let rate_limit_warning =
                if let Some(state) = db::get_rate_limit_state(&conn, &forge_type)? {
                    if let Some(reset_at) = state.reset_at {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64;
                        if now < reset_at && state.last_error.is_some() {
                            let reset_time = chrono::DateTime::from_timestamp(reset_at, 0)
                                .map(|dt| {
                                    use chrono::Local;
                                    let local: chrono::DateTime<Local> = dt.into();
                                    local.format("%-I:%M %p").to_string()
                                })
                                .unwrap_or_else(|| format!("{}s", reset_at - now));
                            format!(" ⚠️  rate limited until {}", reset_time)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

            println!("  {} [{}]", display, forge_type);
            println!("    {}{}{}", sync_info, pending_info, rate_limit_warning);
        }
    }

    Ok(())
}

pub fn cmd_start() -> Result<()> {
    service::start()?;
    println!("✓ Service started");
    Ok(())
}

pub fn cmd_stop() -> Result<()> {
    service::stop()?;
    println!("✓ Service stopped");
    Ok(())
}

pub fn cmd_watch() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    db::add_watched_repo(&conn, &repo_path)?;
    println!("✓ Watching {} ({})", link.forge_repo, repo_path);
    Ok(())
}

pub fn cmd_unwatch() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;
    db::remove_watched_repo(&conn, &repo_path)?;
    println!("✓ Stopped watching {}", repo_path);
    Ok(())
}
