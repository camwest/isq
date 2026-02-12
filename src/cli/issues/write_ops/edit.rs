//! Issue edit command

use std::time::Instant;

use anyhow::Result;

use crate::db;
use crate::display;
use crate::forges::{UpdateIssueRequest, get_forge_for_repo};
use crate::repo;

use crate::cli::utils::{
    WriteResult, is_offline_error, parse_forge_repo, read_stdin_if_piped,
    validate_jira_issue_prefix,
};

pub async fn cmd_edit(
    id: &str,
    title: Option<String>,
    body: Option<String>,
    priority: Option<u8>,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let body = resolve_body_input(body)?;
    validate_update_fields(&title, &body, priority)?;

    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let repo_struct = parse_forge_repo(&link.forge_repo)?;
    validate_jira_issue_prefix(id, &repo_struct.name, &link.forge_type)?;

    let issue_display = display::format_issue_id(id);
    let req = UpdateIssueRequest {
        title: title.clone(),
        body: body.clone(),
        priority,
    };

    match forge.update_issue(&repo_struct, id, req).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_id: Some(id.to_string()),
                    message: format!("Updated {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!("✓ Updated {} ({:.0}ms)", issue_display, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_id": id,
                "title": title,
                "body": body,
                "priority": priority,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "edit", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_id: Some(id.to_string()),
                    message: format!("Queued: edit {}", issue_display),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else if !quiet {
                println!(
                    "✓ Queued: edit {} (offline, {:.0}ms)",
                    issue_display,
                    elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

fn resolve_body_input(body: Option<String>) -> Result<Option<String>> {
    match body {
        Some(value) if value == "-" => {
            let piped = read_stdin_if_piped()?.ok_or_else(|| {
                anyhow::anyhow!(
                    "--body - requires stdin input.\n\
                     Usage: echo \"text\" | isq issue edit <ID> --body -"
                )
            })?;
            Ok(Some(piped))
        }
        Some(value) => Ok(Some(value)),
        None => Ok(None),
    }
}

fn validate_update_fields(
    title: &Option<String>,
    body: &Option<String>,
    priority: Option<u8>,
) -> Result<()> {
    if title.is_none() && body.is_none() && priority.is_none() {
        anyhow::bail!("No fields to edit. Provide at least one of --title, --body, or --priority.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{resolve_body_input, validate_update_fields};

    #[test]
    fn validate_update_fields_requires_at_least_one_field() {
        let err = validate_update_fields(&None, &None, None).unwrap_err();
        assert!(err.to_string().contains("No fields to edit"));
    }

    #[test]
    fn validate_update_fields_accepts_any_field() {
        assert!(validate_update_fields(&Some("x".to_string()), &None, None).is_ok());
        assert!(validate_update_fields(&None, &Some("x".to_string()), None).is_ok());
        assert!(validate_update_fields(&None, &None, Some(2)).is_ok());
    }

    #[test]
    fn resolve_body_input_keeps_literal_body() {
        let body = resolve_body_input(Some("hello".to_string())).expect("body should parse");
        assert_eq!(body, Some("hello".to_string()));
    }
}
