mod cli;
mod config;
mod credentials;
mod daemon;
mod db;
mod display;
mod forges;
mod install;
mod pager;
mod repo;
mod service;
mod user_config;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands, DaemonCommands, GoalCommands, InstallCommands, IssueCommands, LabelCommands, ViewCommands};

#[tokio::main]
async fn main() -> Result<()> {
    // Migrate credentials from OS keychain to file storage (one-time, silent on no credentials)
    if let Err(e) = credentials::migrate_from_keyring() {
        eprintln!("Warning: Failed to migrate credentials from keychain: {}", e);
    }

    let cli = Cli::parse();

    // Honor --no-color flag
    if cli.no_color {
        colored::control::set_override(false);
    }

    match cli.command {
        None => cli::worktree::cmd_home()?,
        Some(Commands::Link { forge, opt }) => {
            cli::link::cmd_link(forge.as_deref(), opt).await?
        }
        Some(Commands::Unlink) => cli::link::cmd_unlink()?,
        Some(Commands::Logout { forge }) => cli::link::cmd_logout(forge.as_deref())?,
        Some(Commands::Status) => cli::status::cmd_status()?,
        Some(Commands::Issue { command }) => match command {
            IssueCommands::List {
                view,
                id,
                label,
                state,
                all,
                mine,
                unassigned,
                open,
                goal,
                sort,
                opt,
                json,
            } => {
                cli::issues::cmd_list(view, id, label, state, all, mine, unassigned, open, goal, sort, opt, json)
                    .await?
            }
            IssueCommands::Show { id, json } => cli::issues::cmd_show(&id, json)?,
            IssueCommands::Create {
                title,
                body,
                label,
                goal,
                opt,
                json,
            } => cli::issues::cmd_create(title, body, label, goal, opt, json, cli.quiet).await?,
            IssueCommands::Comment { id, message, json } => {
                cli::issues::cmd_comment(&id, message, json, cli.quiet).await?
            }
            IssueCommands::Close { id, json } => cli::issues::cmd_close(&id, json, cli.quiet).await?,
            IssueCommands::Reopen { id, json } => cli::issues::cmd_reopen(&id, json, cli.quiet).await?,
            IssueCommands::Label {
                id,
                action,
                label,
                json,
            } => cli::issues::cmd_label(&id, action, label, json, cli.quiet).await?,
            IssueCommands::Assign { id, user, json } => {
                cli::issues::cmd_assign(&id, user, json, cli.quiet).await?
            }
        },
        Some(Commands::Daemon { command }) => match command {
            DaemonCommands::Status => cli::daemon::cmd_status()?,
            DaemonCommands::Start => cli::daemon::cmd_start()?,
            DaemonCommands::Stop => cli::daemon::cmd_stop()?,
            DaemonCommands::Watch => cli::daemon::cmd_watch()?,
            DaemonCommands::Unwatch => cli::daemon::cmd_unwatch()?,
            DaemonCommands::Run => daemon::run_loop().await?,
        },
        Some(Commands::Sync) => cli::sync::cmd_sync(cli.quiet).await?,
        Some(Commands::Goal { command }) => match command {
            GoalCommands::List { state, json } => cli::goals::cmd_list(state, json).await?,
            GoalCommands::Show { name, json } => cli::goals::cmd_show(name, json)?,
            GoalCommands::Create {
                name,
                target,
                body,
                json,
            } => cli::goals::cmd_create(name, target, body, json, cli.quiet).await?,
            GoalCommands::Assign { issue, goal, json } => {
                cli::goals::cmd_assign(&issue, goal, json, cli.quiet).await?
            }
            GoalCommands::Close { name, json } => cli::goals::cmd_close(name, json, cli.quiet).await?,
        },
        Some(Commands::Current { quiet }) => cli::worktree::cmd_current(quiet)?,
        Some(Commands::Start { id }) => cli::worktree::cmd_start(id).await?,
        Some(Commands::Cleanup { keep }) => cli::worktree::cmd_cleanup(keep).await?,
        Some(Commands::Label { command }) => match command {
            LabelCommands::List { json } => cli::labels::cmd_list(json).await?,
            LabelCommands::Create {
                name,
                color,
                description,
                json,
            } => cli::labels::cmd_create(name, color, description, json, cli.quiet).await?,
        },
        Some(Commands::Forge { forge, args }) => cli::forge::cmd_forge(forge, args).await?,
        Some(Commands::View { command }) => match command {
            ViewCommands::Create {
                name,
                label,
                label_not,
                label_any,
                state,
                mine,
                unassigned,
                goal,
                priority,
                priority_lte,
                priority_gte,
                updated_before,
                updated_after,
                created_before,
                created_after,
                sort,
            } => cli::views::cmd_create(
                name,
                label,
                label_not,
                label_any,
                state,
                mine,
                unassigned,
                goal,
                priority,
                priority_lte,
                priority_gte,
                updated_before,
                updated_after,
                created_before,
                created_after,
                sort,
            )?,
            ViewCommands::List { json } => cli::views::cmd_list(json)?,
            ViewCommands::Show { name, json } => cli::views::cmd_show(&name, json)?,
            ViewCommands::Delete { name } => cli::views::cmd_delete(&name)?,
        },
        Some(Commands::Install { command }) => match command {
            InstallCommands::WriteReceipt {
                method,
                binary_path,
                auto_update,
            } => cli::install::cmd_write_receipt(method, binary_path, auto_update)?,
        },
    }

    Ok(())
}
