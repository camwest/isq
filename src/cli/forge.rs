//! Forge-specific CLI commands

use anyhow::Result;

use crate::forges::{ALL_FORGE_TYPES, ForgeType, get_forge_for_repo};
use crate::repo;

pub async fn cmd_forge(forge_name: String, args: Vec<String>) -> Result<()> {
    // Parse forge type
    let forge_type = ForgeType::from_str(&forge_name).ok_or_else(|| {
        let forges: Vec<_> = ALL_FORGE_TYPES.iter().map(|f| f.as_str()).collect();
        anyhow::anyhow!(
            "Unknown forge: {}\n\nAvailable forges: {}",
            forge_name,
            forges.join(", ")
        )
    })?;

    // Get command from args
    let command = args.first().ok_or_else(|| {
        let commands = forge_type.available_commands();
        if commands.is_empty() {
            anyhow::anyhow!("No forge-specific commands available for {}", forge_name)
        } else {
            anyhow::anyhow!(
                "Missing command.\n\nAvailable commands for {}:\n  {}",
                forge_name,
                commands.join("\n  ")
            )
        }
    })?;

    let remaining_args: Vec<String> = args.iter().skip(1).cloned().collect();

    // Get repo path and forge client
    let repo_path = repo::detect_repo_path()?;
    let (forge, _link) = get_forge_for_repo(&repo_path)?;

    // Dispatch to forge
    forge.handle_command(command, &remaining_args).await
}
