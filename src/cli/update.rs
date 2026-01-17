//! Update command handlers

use anyhow::Result;
use serde_json::json;

use crate::install::{self, InstallMethod};
use crate::updater;

pub async fn cmd_check(json: bool) -> Result<()> {
    let json = crate::user_config::resolve_json_default(json)?;
    let receipt = install::read_receipt()?;

    match updater::check_for_updates().await? {
        Some(info) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                println!("Current version: {}", info.current_version);
                println!("Latest version:  {}", info.latest_version);
                println!("Published:       {}", info.published_at);

                if let Some(notes) = &info.release_notes {
                    if !notes.is_empty() {
                        println!("\nRelease notes:");
                        let lines: Vec<_> = notes.lines().collect();
                        for line in lines.iter().take(10) {
                            println!("  {}", line);
                        }
                        if lines.len() > 10 {
                            println!("  ...");
                        }
                    }
                }

                println!();
                let update_cmd = match receipt.as_ref().map(|r| &r.install_method) {
                    Some(InstallMethod::Homebrew) => "brew upgrade isq",
                    Some(InstallMethod::Scoop) => "scoop update isq",
                    Some(InstallMethod::Cargo) => "cargo install isq",
                    _ => "isq update install",
                };
                println!("Run `{}` to update.", update_cmd);
            }
        }
        None => {
            let current = env!("CARGO_PKG_VERSION");
            if json {
                println!("{}", json!({"up_to_date": true, "version": current}));
            } else {
                println!("You're on the latest version ({})", current);
            }
        }
    }
    Ok(())
}
