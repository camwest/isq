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

    // Load parent issue if this issue has a parent
    let parent_issue = if let Some(ref issue) = issue
        && let Some(ref parent_id) = issue.parent_id
    {
        db::load_issue(&conn, &link.forge_repo, parent_id)?
    } else {
        None
    };

    // Load children issues
    let children = if issue.is_some() {
        let filter = db::IssueFilter {
            children_of: Some(issue_id),
            ..Default::default()
        };
        db::load_issues_with_filter(&conn, &link.forge_repo, &filter)?
    } else {
        vec![]
    };

    let elapsed = start.elapsed();

    match issue {
        Some(issue) => {
            if json_output {
                // Include comments, parent, and children in JSON output
                let output = serde_json::json!({
                    "issue": issue,
                    "parent": parent_issue,
                    "children": children,
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
                let output = display::format_issue_with_hierarchy(
                    &issue,
                    &comments,
                    &parent_issue,
                    &children,
                );
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

/// Print issues in a hierarchical tree format
pub(crate) fn print_issues_tree(issues: &[Issue], comment_counts: &HashMap<String, usize>) {
    if issues.is_empty() {
        println!("No open issues.");
        return;
    }

    // Build a map of parent_id -> children
    let mut children_map: HashMap<Option<String>, Vec<&Issue>> = HashMap::new();
    for issue in issues {
        children_map
            .entry(issue.parent_id.clone())
            .or_default()
            .push(issue);
    }

    // Collect all issue IDs in this result set
    let issue_ids: std::collections::HashSet<&str> = issues.iter().map(|i| i.id.as_str()).collect();

    // Print root issues (those without a parent, or whose parent is not in the result set)
    let roots: Vec<&Issue> = issues
        .iter()
        .filter(|i| {
            i.parent_id.is_none() || !issue_ids.contains(i.parent_id.as_ref().unwrap().as_str())
        })
        .collect();

    for root in roots {
        print_issue_tree_node(root, &children_map, comment_counts, 0);
    }
}

fn print_issue_tree_node(
    issue: &Issue,
    children_map: &HashMap<Option<String>, Vec<&Issue>>,
    comment_counts: &HashMap<String, usize>,
    depth: usize,
) {
    let count = comment_counts.get(&issue.id).copied();
    display::print_issue_row_indented(issue, count, depth);

    // Print children
    if let Some(children) = children_map.get(&Some(issue.id.clone())) {
        for child in children {
            print_issue_tree_node(child, children_map, comment_counts, depth + 1);
        }
    }
}
