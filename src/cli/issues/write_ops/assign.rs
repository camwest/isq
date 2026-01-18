//! Issue assignment command

use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::display;
use crate::forges::get_forge_for_repo;
use crate::repo;

use crate::cli::utils::{
    WriteResult, is_offline_error, parse_forge_repo, validate_jira_issue_prefix,
};

pub async fn cmd_assign(id: &str, user: String, json: bool, cli_quiet: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

    let issue_display = display::format_issue_id(id);

    match forge.assign_issue(&repo_struct, id, &user).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(id.to_string()),
                    message: format!("Assigned @{} to {}", user, issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Assigned @{} to {} ({:.0}ms)",
                    user,
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_id": id,
                "assignee": user,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "assign", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(id.to_string()),
                    message: format!("Queued: assign @{} to {}", user, issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: assign @{} to {} (offline, {:.0}ms)",
                    user,
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}
