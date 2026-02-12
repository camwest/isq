// Windows is not supported. See README for supported platforms.
#[cfg(target_os = "windows")]
compile_error!("Windows is not supported. isq runs on macOS and Linux only.");

mod cli;
mod config;
mod credentials;
mod daemon;
mod db;
mod display;
mod forges;
mod install;
mod logging;
mod pager;
mod repo;
mod service;
mod updater;
mod user_config;

use anyhow::Result;
use clap::Parser;

use crate::cli::{
    Cli, Commands, DaemonCommands, GoalCommands, InstallCommands, IssueCommands, LabelCommands,
    UpdateCommands, ViewCommands,
};

/// Check if a command depends on the daemon being current.
fn should_check_daemon_version(command: &Option<Commands>) -> bool {
    matches!(
        command,
        Some(
            Commands::Daemon {
                command: DaemonCommands::Status
                    | DaemonCommands::Start
                    | DaemonCommands::Watch
                    | DaemonCommands::Unwatch
            } | Commands::Sync
                | Commands::Issue { .. }
                | Commands::Goal { .. }
                | Commands::Label { .. }
        )
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    // Check for staged update before anything else
    // If a staged update is ready, apply it and restart
    if let Ok(Some(staged)) = updater::check_staged_update() {
        if let Err(e) = updater::apply_staged_update(&staged) {
            eprintln!("Warning: Failed to apply staged update: {}", e);
            updater::cleanup_staged_update();
        } else {
            // Binary replaced - restart to run new version
            return updater::restart_self();
        }
    }

    // Migrate credentials from OS keychain to file storage (one-time, silent on no credentials)
    if let Err(e) = credentials::migrate_from_keyring() {
        eprintln!(
            "Warning: Failed to migrate credentials from keychain: {}",
            e
        );
    }

    let cli = Cli::parse();

    // Initialize logging based on mode:
    // - Daemon run mode: log to file with rotation
    // - All other commands: log to stderr
    // (ISQ_LOG env var can override level, see logging module)
    let _log_guard = if matches!(
        cli.command,
        Some(Commands::Daemon {
            command: DaemonCommands::Run
        })
    ) {
        // Daemon mode: log to file, default to INFO level
        let verbosity = if cli.verbose > 0 { cli.verbose } else { 1 };
        logging::init_daemon(verbosity)
    } else {
        // CLI mode: log to stderr
        logging::init_cli(cli.verbose);
        None
    };

    // Spawn background update check (non-blocking)
    // This runs in a separate task and won't slow down CLI startup
    updater::maybe_check_for_updates_background();

    // Honor --no-color flag
    if cli.no_color {
        colored::control::set_override(false);
    }

    // Handle --version before command routing
    if cli.version {
        cli::print_version();
        return Ok(());
    }

    // Check if daemon needs restart for new version before running daemon-dependent commands
    if should_check_daemon_version(&cli.command)
        && let Ok(true) = cli::daemon::ensure_daemon_current()
    {
        eprintln!("Daemon restarted for new version");
    }

    match cli.command {
        None => cli::worktree::cmd_home()?,
        Some(Commands::Link { forge, opt }) => cli::link::cmd_link(forge.as_deref(), opt).await?,
        Some(Commands::Unlink) => cli::link::cmd_unlink()?,
        Some(Commands::Logout { forge }) => cli::link::cmd_logout(forge.as_deref())?,
        Some(Commands::Status) => cli::status::cmd_status()?,
        Some(Commands::Doctor {
            verbose,
            json,
            check,
        }) => cli::doctor::cmd_doctor(verbose, json, check.as_deref())?,
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
                tree,
                flat,
                root_only,
                children_of,
                opt,
                json,
            } => {
                cli::issues::cmd_list(
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
                    tree,
                    flat,
                    root_only,
                    children_of,
                    opt,
                    json,
                )
                .await?
            }
            IssueCommands::Show { id, json } => cli::issues::cmd_show(&id, json)?,
            IssueCommands::Create {
                title,
                body,
                label,
                goal,
                parent,
                opt,
                json,
            } => {
                cli::issues::cmd_create(title, body, label, goal, parent, opt, json, cli.quiet)
                    .await?
            }
            IssueCommands::Comment { id, message, json } => {
                cli::issues::cmd_comment(&id, message, json, cli.quiet).await?
            }
            IssueCommands::Close { id, json } => {
                cli::issues::cmd_close(&id, json, cli.quiet).await?
            }
            IssueCommands::Reopen { id, json } => {
                cli::issues::cmd_reopen(&id, json, cli.quiet).await?
            }
            IssueCommands::Edit {
                id,
                title,
                body,
                priority,
                json,
            } => cli::issues::cmd_edit(&id, title, body, priority, json, cli.quiet).await?,
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
            DaemonCommands::Logs { lines, follow } => cli::daemon::cmd_logs(lines, follow)?,
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
            GoalCommands::Close { name, json } => {
                cli::goals::cmd_close(name, json, cli.quiet).await?
            }
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
                tree,
                flat,
                root_only,
                children_of,
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
                tree,
                flat,
                root_only,
                children_of,
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
        Some(Commands::Update { command }) => match command {
            UpdateCommands::Check { json } => cli::update::cmd_check(json).await?,
            UpdateCommands::Install => cli::update::cmd_install().await?,
        },
        Some(Commands::Uninstall {
            keep_config,
            keep_cache,
            yes,
            dry_run,
        }) => cli::uninstall::cmd_uninstall(keep_config, keep_cache, yes, dry_run)?,
    }

    Ok(())
}
