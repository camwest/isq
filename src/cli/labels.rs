//! Label-related CLI commands

use std::time::Instant;

use anyhow::Result;

use crate::forges::get_forge_for_repo;
use crate::repo;

pub async fn cmd_list(json_output: bool) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json_output = crate::user_config::resolve_json_default(json_output)?;

    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let labels = forge.list_labels(&repo_struct).await?;
    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&labels)?);
    } else {
        if labels.is_empty() {
            println!("No labels found.");
        } else {
            for label in &labels {
                if let Some(color) = &label.color {
                    println!("  {} ({})", label.name, color);
                } else {
                    println!("  {}", label.name);
                }
            }
            eprintln!("\n{} labels in {:.0}ms", labels.len(), elapsed.as_millis());
        }
    }

    Ok(())
}

pub async fn cmd_create(
    name: String,
    color: Option<String>,
    description: Option<String>,
    json: bool,
    cli_quiet: bool,
) -> Result<()> {
    // Apply json default from user config (CLI flag overrides)
    let json = crate::user_config::resolve_json_default(json)?;
    // Resolve quiet setting (CLI flag overrides config)
    let quiet = crate::user_config::resolve_quiet_default(cli_quiet)?;

    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo_struct = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let label = forge
        .create_label(
            &repo_struct,
            &name,
            color.as_deref(),
            description.as_deref(),
        )
        .await?;
    let elapsed = start.elapsed();

    if json {
        println!("{}", serde_json::to_string_pretty(&label)?);
    } else if !quiet {
        if let Some(color) = &label.color {
            println!(
                "✓ Created label '{}' ({}) in {:.0}ms",
                label.name,
                color,
                elapsed.as_millis()
            );
        } else {
            println!(
                "✓ Created label '{}' in {:.0}ms",
                label.name,
                elapsed.as_millis()
            );
        }
    }

    Ok(())
}
