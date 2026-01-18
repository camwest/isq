//! Status CLI command

use anyhow::Result;

use crate::db;
use crate::forges::ALL_FORGE_TYPES;
use crate::repo;
use crate::service;

pub fn cmd_status() -> Result<()> {
    // Auth status
    println!("Authentication:");

    for forge_type in ALL_FORGE_TYPES {
        let auth = forge_type.auth();
        print!("  {:10}", auth.display_name);
        if auth.has_credentials() {
            println!("ready");
        } else {
            println!("not configured (run: {})", auth.link_command);
        }
    }

    // Current repo link (if in a git repo)
    println!();
    match repo::detect_repo_path() {
        Ok(repo_path) => {
            let conn = db::open()?;
            match db::get_repo_link(&conn, &repo_path)? {
                Some(link) => {
                    let display = link.display_name.as_deref().unwrap_or(&link.forge_repo);
                    println!("This repo:");
                    println!("  Linked to {} ({})", display, link.forge_type);

                    // Show sync state
                    if let Some(sync_state) = db::get_sync_state(&conn, &link.forge_repo)? {
                        let last_sync = sync_state.last_sync.as_deref().unwrap_or("never");
                        println!("  {} issues cached ({})", sync_state.issue_count, last_sync);
                    }

                    // Show pending ops
                    let pending = db::count_pending_ops(&conn, &link.forge_repo)?;
                    if pending > 0 {
                        println!("  {} pending operations", pending);
                    }

                    // Show rate limit status
                    if let Some(state) = db::get_rate_limit_state(&conn, &link.forge_type)?
                        && let Some(reset_at) = state.reset_at
                    {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs() as i64;
                        if now < reset_at {
                            let wait_secs = reset_at - now;
                            // Convert to local time, 12-hour format like macOS default
                            let reset_time = chrono::DateTime::from_timestamp(reset_at, 0)
                                .map(|dt| {
                                    use chrono::Local;
                                    let local: chrono::DateTime<Local> = dt.into();
                                    local.format("%-I:%M %p").to_string()
                                })
                                .unwrap_or_else(|| format!("{}s", wait_secs));
                            println!("  ⚠️  Rate limited until {}", reset_time);
                        }
                    }
                }
                None => {
                    println!("This repo:");
                    println!("  Not linked (run: isq link <forge>)");
                }
            }
        }
        Err(_) => {
            println!("Not in a git repository");
        }
    }

    // Service status
    println!();
    print!("Service:    ");
    let svc_status = service::status()?;
    if !svc_status.installed {
        println!("not installed");
    } else if let Some(pid) = svc_status.pid {
        println!("running (PID {})", pid);
    } else {
        println!("installed but not running");
    }

    Ok(())
}
