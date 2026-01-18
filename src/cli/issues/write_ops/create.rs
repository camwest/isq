//! Issue creation command

use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::display;
use crate::forges::{CreateIssueRequest, get_forge_for_repo};
use crate::repo;

use crate::cli::utils::{WriteResult, is_offline_error, parse_forge_repo};

#[allow(clippy::too_many_arguments)]
pub async fn cmd_create(
    title: String,
    body: Option<String>,
    labels: Vec<String>,
    goal: Option<String>,
    parent: Option<String>,
    opts: Vec<String>,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Resolve body: explicit arg takes precedence, then stdin
    let body = body.or(crate::cli::utils::read_stdin_if_piped()?);

    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();

    // Parse forge-specific options
    let opts = crate::forges::parse_opts(&opts);

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;
    let conn = db::open()?;

    // Resolve goal name to goal_id if provided
    let goal_id = if let Some(goal_name) = &goal {
        let g = db::load_goal_by_name(&conn, &link.forge_repo, goal_name)?.ok_or_else(|| {
            anyhow::anyhow!("Goal '{}' not found. Run `isq sync` to refresh.", goal_name)
        })?;
        Some(g.id)
    } else {
        None
    };

    let repo_struct = parse_forge_repo(&link.forge_repo)?;

    let req = CreateIssueRequest {
        title: title.clone(),
        body: body.clone(),
        labels: labels.clone(),
        goal_id: goal_id.clone(),
        parent_id: parent.clone(),
        opts: opts.clone(),
    };

    match forge.create_issue(&repo_struct, req).await {
        Ok(issue) => {
            let elapsed = start.elapsed();
            let id_display = display::format_issue_id(&issue.id);
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(issue.id.clone()),
                    message: format!("Created {} {}", id_display, issue.title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Created {} {} ({:.0}ms)",
                    id_display,
                    issue.title,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "title": title,
                "body": body,
                "labels": labels,
                "goal_id": goal_id,
                "parent_id": parent,
            });
            db::queue_op(&conn, &link.forge_repo, "create", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: None,
                    message: format!("Queued: {}", title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: {} (offline, {:.0}ms)",
                    title,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}
