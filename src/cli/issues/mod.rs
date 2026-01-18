//! Issue-related CLI commands

mod list;
mod write_ops;

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::display;
use crate::forges::{Issue, not_linked_error};
use crate::pager;
use crate::repo;

// Re-export all public commands
pub use list::cmd_list;
pub use write_ops::{cmd_assign, cmd_close, cmd_comment, cmd_create, cmd_label, cmd_reopen};

pub fn cmd_show(id: &str, json_output: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json_output = crate::user_config::resolve_json_default(json_output)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?.ok_or_else(not_linked_error)?;

    // For JIRA, validate issue key prefix matches linked project
    if link.forge_type == "jira" {
        let project_key = link.forge_repo.split('/').next_back().unwrap_or("");
        if id.contains('-') {
            let prefix = id.split('-').next().unwrap_or("");
            if !prefix.eq_ignore_ascii_case(project_key) {
                anyhow::bail!(
                    "Issue '{}' belongs to project '{}', but you're linked to '{}'.",
                    id,
                    prefix,
                    project_key
                );
            }
        }
    }
    let issue_id = id;

    // Touch repo to update last_accessed for daemon priority
    db::touch_repo(&conn, &repo_path)?;

    let issue = db::load_issue(&conn, &link.forge_repo, issue_id)?;
    let comments = db::load_comments(&conn, &link.forge_repo, issue_id)?;
    let elapsed = start.elapsed();

    match issue {
        Some(issue) => {
            if json_output {
                // Include comments in JSON output
                let output = serde_json::json!({
                    "issue": issue,
                    "comments": comments.iter().map(|c| {
                        serde_json::json!({
                            "id": c.comment_id,
                            "body": c.body,
                            "author": c.author,
                            "created_at": c.created_at
                        })
                    }).collect::<Vec<_>>()
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                // Format issue and pipe through pager if needed
                let output = display::format_issue(&issue, &comments);
                pager::print_with_pager(&output);

                // Timing footer (to stderr, outside pager)
                display::print_timing_footer(elapsed.as_millis() as u64);
            }
        }
        None => {
            anyhow::bail!(
                "Issue {} not found in cache. Run `isq sync` to refresh.",
                id
            );
        }
    }

    Ok(())
}

pub(crate) fn print_issues(issues: &[Issue], comment_counts: &HashMap<String, usize>) {
    if issues.is_empty() {
        println!("No open issues.");
        return;
    }

    for issue in issues {
        let count = comment_counts.get(&issue.id).copied();
        display::print_issue_row(issue, count);
    }
}
