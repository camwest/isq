//! Status CLI command

use anyhow::Result;

use crate::db::{self, SyncHealth};
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

    // Service status (get early for health check)
    let svc_status = service::status()?;
    let daemon_running = svc_status.pid.is_some();

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

                    // Get sync and rate limit state for health calculation
                    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
                    let rate_limit_state = db::get_rate_limit_state(&conn, &link.forge_type)?;

                    // Show sync state
                    if let Some(ref state) = sync_state {
                        let last_sync = state
                            .last_full_sync_at
                            .as_deref()
                            .or(state.issues_last_sync.as_deref())
                            .or(state.last_sync.as_deref())
                            .unwrap_or("never");
                        println!(
                            "  {} issues cached (last sync: {})",
                            state.issue_count, last_sync
                        );
                    }

                    // Show pending ops
                    let pending = db::count_pending_ops(&conn, &link.forge_repo)?;
                    if pending > 0 {
                        println!("  {} pending operations", pending);
                    }

                    // Calculate and display sync health
                    let health = db::calculate_sync_health(
                        sync_state.as_ref(),
                        rate_limit_state.as_ref(),
                        daemon_running,
                    );

                    match health {
                        SyncHealth::Healthy => {
                            println!("  Sync: healthy");
                        }
                        SyncHealth::Degraded { reason, guidance } => {
                            println!("  Sync: degraded - {}", reason);
                            println!("        {}", guidance);
                        }
                        SyncHealth::Unhealthy { reason, guidance } => {
                            println!("  Sync: unhealthy - {}", reason);
                            println!("        {}", guidance);
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
    if !svc_status.installed {
        println!("not installed");
    } else if let Some(pid) = svc_status.pid {
        println!("running (PID {})", pid);
    } else {
        println!("installed but not running");
    }

    Ok(())
}
