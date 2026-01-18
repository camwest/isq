//! Uninstall command - guided removal of isq and its components

use anyhow::Result;
use colored::Colorize;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::{install, service, user_config};

/// Information about what will be uninstalled
struct UninstallItems {
    binary_path: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    service_installed: bool,
    daemon_running: bool,
}

/// Get the cache directory path
fn cache_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "isq")
        .ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
    Ok(dirs.cache_dir().to_path_buf())
}

/// Detect what isq components exist on the system
fn detect_components() -> UninstallItems {
    let binary_path = install::read_receipt()
        .ok()
        .flatten()
        .map(|r| r.binary_path);

    let config_dir = user_config::config_dir().ok().filter(|p| p.exists());

    let cache_dir = cache_dir().ok().filter(|p| p.exists());

    let (service_installed, daemon_running) = service::status()
        .map(|s| (s.installed, s.running))
        .unwrap_or((false, false));

    UninstallItems {
        binary_path,
        config_dir,
        cache_dir,
        service_installed,
        daemon_running,
    }
}

/// Print what will be uninstalled
fn print_uninstall_plan(items: &UninstallItems, keep_config: bool, keep_cache: bool) {
    println!("This will remove isq and its associated files:\n");

    if let Some(path) = &items.binary_path {
        println!("  {}    {}", "Binary:".bold(), path.display());
    } else {
        println!(
            "  {}    {} (not found or not tracked)",
            "Binary:".bold(),
            "?".dimmed()
        );
    }

    if let Some(path) = &items.config_dir {
        if keep_config {
            println!(
                "  {}    {} {}",
                "Config:".bold(),
                path.display(),
                "(keeping)".dimmed()
            );
        } else {
            println!(
                "  {}    {} (contains views, credentials)",
                "Config:".bold(),
                path.display()
            );
        }
    }

    if let Some(path) = &items.cache_dir {
        if keep_cache {
            println!(
                "  {}     {} {}",
                "Cache:".bold(),
                path.display(),
                "(keeping)".dimmed()
            );
        } else {
            println!(
                "  {}     {} (contains issue database)",
                "Cache:".bold(),
                path.display()
            );
        }
    }

    if items.service_installed {
        println!(
            "  {}   installed (will be stopped and removed)",
            "Service:".bold()
        );
    }

    println!();
}

/// Prompt user for confirmation
fn confirm(prompt: &str) -> Result<bool> {
    print!("{} [y/N] ", prompt);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// Main uninstall command
pub fn cmd_uninstall(keep_config: bool, keep_cache: bool, yes: bool, dry_run: bool) -> Result<()> {
    let items = detect_components();

    // Check if there's anything to uninstall
    let has_anything = items.binary_path.is_some()
        || items.config_dir.is_some()
        || items.cache_dir.is_some()
        || items.service_installed;

    if !has_anything {
        println!("Nothing to uninstall. isq does not appear to be installed.");
        return Ok(());
    }

    // Show plan
    print_uninstall_plan(&items, keep_config, keep_cache);

    if dry_run {
        println!("{}", "Dry run - no changes made.".dimmed());
        return Ok(());
    }

    // Confirm
    if !yes && !confirm("Proceed?")? {
        println!("Cancelled.");
        return Ok(());
    }

    println!();

    // Stop daemon if running
    if items.daemon_running {
        print!("Stopping daemon... ");
        io::stdout().flush()?;
        match service::stop() {
            Ok(_) => println!("{}", "done".green()),
            Err(e) => println!("{}: {}", "warning".yellow(), e),
        }
    }

    // Uninstall service
    if items.service_installed {
        print!("Removing service... ");
        io::stdout().flush()?;
        match service::uninstall() {
            Ok(_) => println!("{}", "done".green()),
            Err(e) => println!("{}: {}", "warning".yellow(), e),
        }
    }

    // Remove config directory
    if !keep_config && let Some(dir) = &items.config_dir {
        print!("Removing config... ");
        io::stdout().flush()?;
        match fs::remove_dir_all(dir) {
            Ok(_) => println!("{}", "done".green()),
            Err(e) => println!("{}: {}", "warning".yellow(), e),
        }
    }

    // Remove cache directory
    if !keep_cache && let Some(dir) = &items.cache_dir {
        print!("Removing cache... ");
        io::stdout().flush()?;
        match fs::remove_dir_all(dir) {
            Ok(_) => println!("{}", "done".green()),
            Err(e) => println!("{}: {}", "warning".yellow(), e),
        }
    }

    println!();

    // Binary removal instruction
    if let Some(path) = &items.binary_path {
        if path.exists() {
            let path_str = path.to_string_lossy();
            if path_str.starts_with("/usr") {
                println!(
                    "{} To complete uninstall, run:\n  sudo rm {}",
                    "->".cyan(),
                    path.display()
                );
            } else {
                println!(
                    "{} To complete uninstall, run:\n  rm {}",
                    "->".cyan(),
                    path.display()
                );
            }
        } else {
            println!("{} Binary already removed.", "->".cyan());
        }
    } else {
        // No receipt - guide user to find and remove binary
        println!(
            "{} Binary location unknown. Find it with:\n  which isq",
            "->".cyan()
        );
    }

    // Note about preserved data
    if keep_config || keep_cache {
        println!();
        if keep_config && let Some(dir) = &items.config_dir {
            println!(
                "{} Config preserved at: {}",
                "Note:".dimmed(),
                dir.display()
            );
        }
        if keep_cache && let Some(dir) = &items.cache_dir {
            println!("{} Cache preserved at: {}", "Note:".dimmed(), dir.display());
        }
    }

    Ok(())
}
