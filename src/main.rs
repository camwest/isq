mod auth;
mod daemon;
mod db;
mod forge;
mod github;
mod linear;
mod oauth;
mod repo;

use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::forge::{get_forge, get_forge_for_repo, CreateIssueRequest, Forge, ForgeType};
use crate::github::Issue;

/// Check if an error is a network/connectivity error (offline)
fn is_offline_error(err: &anyhow::Error) -> bool {
    let err_str = err.to_string().to_lowercase();
    err_str.contains("connection refused")
        || err_str.contains("network is unreachable")
        || err_str.contains("no route to host")
        || err_str.contains("dns error")
        || err_str.contains("connection reset")
        || err_str.contains("timed out")
        || err_str.contains("could not resolve")
}

#[derive(Parser)]
#[command(name = "isq")]
#[command(about = "Instant issue tracking. Offline-first. AI-agent native.")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Link this repo to an issue tracker
    Link {
        /// Forge type: github or linear
        forge: String,
    },

    /// Unlink this repo from its issue tracker
    Unlink,

    /// Show status (auth, link, daemon)
    Status,

    /// Issue operations
    Issue {
        #[command(subcommand)]
        command: IssueCommands,
    },

    /// Daemon operations
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Sync issues from remote
    Sync,
}

#[derive(Subcommand)]
enum IssueCommands {
    /// List issues
    List {
        /// Filter by label
        #[arg(long)]
        label: Option<String>,

        /// Filter by state (open, closed)
        #[arg(long)]
        state: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show a single issue
    Show {
        /// Issue number
        id: u64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new issue
    Create {
        /// Issue title
        #[arg(long)]
        title: String,

        /// Issue body
        #[arg(long)]
        body: Option<String>,

        /// Labels to add
        #[arg(long)]
        label: Vec<String>,
    },

    /// Add a comment to an issue
    Comment {
        /// Issue number
        id: u64,

        /// Comment body
        message: String,
    },

    /// Close an issue
    Close {
        /// Issue number
        id: u64,
    },

    /// Reopen an issue
    Reopen {
        /// Issue number
        id: u64,
    },

    /// Manage labels on an issue
    Label {
        /// Issue number
        id: u64,

        /// Action: add or remove
        action: String,

        /// Label name
        label: String,
    },

    /// Assign a user to an issue
    Assign {
        /// Issue number
        id: u64,

        /// Username to assign
        user: String,
    },
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Show daemon status and watched repos
    Status,

    /// Start the daemon
    Start,

    /// Stop the daemon
    Stop,

    /// Add current repo to watch list
    Watch,

    /// Remove current repo from watch list
    Unwatch,

    /// Run the sync loop (internal, called by spawn)
    #[command(hide = true)]
    Run,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Link { forge } => cmd_link(&forge).await?,
        Commands::Unlink => cmd_unlink()?,
        Commands::Status => cmd_status()?,
        Commands::Issue { command } => match command {
            IssueCommands::List { label, state, json } => cmd_issue_list(label, state, json).await?,
            IssueCommands::Show { id, json } => cmd_issue_show(id, json)?,
            IssueCommands::Create { title, body, label } => {
                cmd_issue_create(title, body, label).await?
            }
            IssueCommands::Comment { id, message } => cmd_issue_comment(id, message).await?,
            IssueCommands::Close { id } => cmd_issue_close(id).await?,
            IssueCommands::Reopen { id } => cmd_issue_reopen(id).await?,
            IssueCommands::Label { id, action, label } => {
                cmd_issue_label(id, action, label).await?
            }
            IssueCommands::Assign { id, user } => cmd_issue_assign(id, user).await?,
        },
        Commands::Daemon { command } => match command {
            DaemonCommands::Status => cmd_daemon_status()?,
            DaemonCommands::Start => cmd_daemon_start()?,
            DaemonCommands::Stop => cmd_daemon_stop()?,
            DaemonCommands::Watch => cmd_daemon_watch()?,
            DaemonCommands::Unwatch => cmd_daemon_unwatch()?,
            DaemonCommands::Run => daemon::run_loop().await?,
        },
        Commands::Sync => cmd_sync().await?,
    }

    Ok(())
}

async fn cmd_link(forge_name: &str) -> Result<()> {
    let forge_type = ForgeType::from_str(forge_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown forge: {}. Supported: github, linear", forge_name))?;

    let repo_path = repo::detect_repo_path()?;

    match forge_type {
        ForgeType::GitHub => {
            // Get GitHub repo from git remote
            let repo = repo::detect_repo()?;
            let forge = get_forge()?;

            // Verify authentication
            let username = forge.get_user().await?;
            println!("✓ Authenticated as {} (via gh CLI)", username);

            // Save the link
            let conn = db::open()?;
            db::set_repo_link(&conn, &repo_path, "github", &repo.full_name())?;

            // Do initial sync
            println!("Syncing {}...", repo.full_name());
            let issues = forge.list_issues(&repo).await?;
            db::save_issues(&conn, &repo.full_name(), &issues)?;

            // Add to watch list (using repo_path as key)
            db::add_watched_repo(&conn, &repo_path)?;

            println!("✓ Cached {} open issues", issues.len());

            // Start daemon if not running
            println!();
            daemon::spawn()?;

            println!("\n✓ Linked to GitHub Issues ({})", repo.full_name());
        }
        ForgeType::Linear => {
            // Run OAuth flow to get token
            let token_response = oauth::linear_oauth_flow().await?;

            // Store the token
            let conn = db::open()?;
            db::set_credential(
                &conn,
                "linear",
                &token_response.access_token,
                token_response.refresh_token.as_deref(),
                None, // TODO: calculate expires_at from expires_in
            )?;

            let client = linear::LinearClient::new(token_response.access_token.clone());

            // Verify authentication and get username
            let username = client.get_viewer().await?;
            println!("✓ Authenticated as {}", username);

            // List teams
            let teams = client.list_teams().await?;
            if teams.is_empty() {
                anyhow::bail!("No teams found in your Linear workspace");
            }

            // Let user pick a team
            println!("\nAvailable teams:");
            for (i, team) in teams.iter().enumerate() {
                println!("  {}. {} ({})", i + 1, team.name, team.key);
            }

            let team = if teams.len() == 1 {
                println!("\nUsing team: {} ({})", teams[0].name, teams[0].key);
                &teams[0]
            } else {
                print!("\nSelect team (1-{}): ", teams.len());
                use std::io::{self, Write};
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let choice: usize = input.trim().parse()
                    .map_err(|_| anyhow::anyhow!("Invalid selection"))?;

                if choice < 1 || choice > teams.len() {
                    anyhow::bail!("Invalid selection");
                }

                &teams[choice - 1]
            };

            // Save the link (use team_id as forge_repo, formatted as "team-key/team-id")
            let forge_repo = format!("{}/{}", team.key, team.id);
            db::set_repo_link(&conn, &repo_path, "linear", &forge_repo)?;

            // Create a pseudo-repo for syncing (owner is team key, name is team id)
            let repo = repo::Repo {
                owner: team.key.clone(),
                name: team.id.clone(),
            };

            // Do initial sync (reuse the client we already created)
            println!("Syncing {}...", team.name);
            let issues = client.list_issues(&repo).await?;
            db::save_issues(&conn, &forge_repo, &issues)?;

            // Add to watch list
            db::add_watched_repo(&conn, &repo_path)?;

            println!("✓ Cached {} open issues", issues.len());

            // Start daemon if not running
            println!();
            daemon::spawn()?;

            println!("\n✓ Linked to Linear ({})", team.name);
        }
    }

    Ok(())
}

fn cmd_unlink() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if linked
    let link = db::get_repo_link(&conn, &repo_path)?;
    if link.is_none() {
        println!("This repo is not linked to any issue tracker.");
        return Ok(());
    }

    let link = link.unwrap();
    db::remove_repo_link(&conn, &repo_path)?;
    db::remove_watched_repo(&conn, &repo_path)?;

    println!("✓ Unlinked from {} ({})", link.forge_type, link.forge_repo);

    Ok(())
}

fn cmd_status() -> Result<()> {
    // Auth status
    println!("Authentication:");

    // GitHub
    print!("  GitHub    ");
    match auth::get_gh_token() {
        Ok(_) => println!("ready (via gh CLI)"),
        Err(_) => println!("not configured (run: gh auth login)"),
    }

    // Linear
    print!("  Linear    ");
    if auth::has_linear_credentials() {
        println!("ready");
    } else {
        println!("not configured (run: isq link linear)");
    }

    // Current repo link (if in a git repo)
    println!();
    match repo::detect_repo_path() {
        Ok(repo_path) => {
            let conn = db::open()?;
            match db::get_repo_link(&conn, &repo_path)? {
                Some(link) => {
                    println!("This repo:");
                    println!("  Linked to {} ({})", link.forge_repo, link.forge_type);

                    // Show sync state
                    if let Some((last_sync, count)) = db::get_sync_state(&conn, &link.forge_repo)? {
                        println!("  {} issues cached ({})", count, last_sync);
                    }

                    // Show pending ops
                    let pending = db::count_pending_ops(&conn, &link.forge_repo)?;
                    if pending > 0 {
                        println!("  {} pending operations", pending);
                    }
                }
                None => {
                    println!("This repo:");
                    println!("  Not linked");
                    println!("  Run: isq link github  or  isq link linear");
                }
            }
        }
        Err(_) => {
            println!("Not in a git repository");
        }
    }

    // Daemon status
    println!();
    print!("Daemon:     ");
    match daemon::is_running()? {
        Some(pid) => println!("running (PID {})", pid),
        None => println!("not running"),
    }

    Ok(())
}

async fn cmd_sync() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    eprintln!("Syncing {}...", link.forge_repo);
    let start = Instant::now();

    let issues = forge.list_issues(&repo).await?;
    let fetch_time = start.elapsed();

    let conn = db::open()?;
    db::save_issues(&conn, &link.forge_repo, &issues)?;

    // Touch repo to update last_accessed
    db::touch_repo(&conn, &repo_path)?;

    println!("✓ Synced {} issues in {:.2}s", issues.len(), fetch_time.as_secs_f64());

    Ok(())
}

async fn cmd_issue_list(
    label: Option<String>,
    state: Option<String>,
    json_output: bool,
) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?
        .ok_or_else(|| anyhow::anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n  isq link github\n  isq link linear"))?;

    // Auto-sync if no cached data
    let sync_state = db::get_sync_state(&conn, &link.forge_repo)?;
    if sync_state.is_none() {
        eprintln!("No cache for {}. Syncing...", link.forge_repo);
        let (forge, _) = get_forge_for_repo(&repo_path)?;

        // Parse forge_repo to create Repo struct
        let parts: Vec<&str> = link.forge_repo.split('/').collect();
        if parts.len() == 2 {
            let repo = repo::Repo {
                owner: parts[0].to_string(),
                name: parts[1].to_string(),
            };
            let issues = forge.list_issues(&repo).await?;
            db::save_issues(&conn, &link.forge_repo, &issues)?;
            eprintln!("✓ Synced {} issues", issues.len());
        }
    }

    // Touch repo to update last_accessed for daemon priority
    db::touch_repo(&conn, &repo_path)?;

    let issues = db::load_issues_filtered(
        &conn,
        &link.forge_repo,
        label.as_deref(),
        state.as_deref(),
    )?;
    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        print_issues(&issues);
        eprintln!("\n{} issues in {:.0}ms", issues.len(), elapsed.as_millis());
    }

    Ok(())
}

fn cmd_issue_show(id: u64, json_output: bool) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?
        .ok_or_else(|| anyhow::anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n  isq link github\n  isq link linear"))?;

    // Touch repo to update last_accessed for daemon priority
    db::touch_repo(&conn, &repo_path)?;

    let issue = db::load_issue(&conn, &link.forge_repo, id)?;
    let elapsed = start.elapsed();

    match issue {
        Some(issue) => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&issue)?);
            } else {
                print_issue_detail(&issue);
                eprintln!("\nLoaded in {:.0}ms", elapsed.as_millis());
            }
        }
        None => {
            anyhow::bail!(
                "Issue #{} not found in cache. Run `isq sync` to refresh.",
                id
            );
        }
    }

    Ok(())
}

async fn cmd_issue_create(title: String, body: Option<String>, labels: Vec<String>) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let req = CreateIssueRequest {
        title: title.clone(),
        body: body.clone(),
        labels: labels.clone(),
    };

    match forge.create_issue(&repo, req).await {
        Ok(issue) => {
            let elapsed = start.elapsed();
            println!(
                "✓ Created #{} {} ({:.0}ms)",
                issue.number, issue.title, elapsed.as_millis()
            );
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "title": title,
                "body": body,
                "labels": labels,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "create", &payload.to_string())?;
            println!(
                "✓ Queued: {} (offline, {:.0}ms)",
                title, elapsed.as_millis()
            );
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_comment(id: u64, message: String) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.create_comment(&repo, id, &message).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            println!("✓ Comment added to #{} ({:.0}ms)", id, elapsed.as_millis());
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_number": id,
                "body": message,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "comment", &payload.to_string())?;
            println!(
                "✓ Queued: comment on #{} (offline, {:.0}ms)",
                id, elapsed.as_millis()
            );
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_close(id: u64) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.close_issue(&repo, id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            println!("✓ Closed #{} ({:.0}ms)", id, elapsed.as_millis());
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_number": id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "close", &payload.to_string())?;
            println!("✓ Queued: close #{} (offline, {:.0}ms)", id, elapsed.as_millis());
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_reopen(id: u64) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.reopen_issue(&repo, id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            println!("✓ Reopened #{} ({:.0}ms)", id, elapsed.as_millis());
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_number": id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "reopen", &payload.to_string())?;
            println!("✓ Queued: reopen #{} (offline, {:.0}ms)", id, elapsed.as_millis());
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_label(id: u64, action: String, label: String) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match action.as_str() {
        "add" => {
            match forge.add_label(&repo, id, &label).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    println!("✓ Added label '{}' to #{} ({:.0}ms)", label, id, elapsed.as_millis());
                }
                Err(e) if is_offline_error(&e) => {
                    let elapsed = start.elapsed();
                    let payload = serde_json::json!({
                        "issue_number": id,
                        "label": label,
                    });
                    let conn = db::open()?;
                    db::queue_op(&conn, &link.forge_repo, "label_add", &payload.to_string())?;
                    println!(
                        "✓ Queued: add label '{}' to #{} (offline, {:.0}ms)",
                        label, id, elapsed.as_millis()
                    );
                }
                Err(e) => return Err(e),
            }
        }
        "remove" => {
            match forge.remove_label(&repo, id, &label).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    println!("✓ Removed label '{}' from #{} ({:.0}ms)", label, id, elapsed.as_millis());
                }
                Err(e) if is_offline_error(&e) => {
                    let elapsed = start.elapsed();
                    let payload = serde_json::json!({
                        "issue_number": id,
                        "label": label,
                    });
                    let conn = db::open()?;
                    db::queue_op(&conn, &link.forge_repo, "label_remove", &payload.to_string())?;
                    println!(
                        "✓ Queued: remove label '{}' from #{} (offline, {:.0}ms)",
                        label, id, elapsed.as_millis()
                    );
                }
                Err(e) => return Err(e),
            }
        }
        _ => {
            anyhow::bail!("Invalid action '{}'. Use 'add' or 'remove'.", action);
        }
    }

    Ok(())
}

async fn cmd_issue_assign(id: u64, user: String) -> Result<()> {
    let start = Instant::now();

    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    // Parse forge_repo to create Repo struct
    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.assign_issue(&repo, id, &user).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            println!("✓ Assigned @{} to #{} ({:.0}ms)", user, id, elapsed.as_millis());
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_number": id,
                "assignee": user,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "assign", &payload.to_string())?;
            println!(
                "✓ Queued: assign @{} to #{} (offline, {:.0}ms)",
                user, id, elapsed.as_millis()
            );
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

fn cmd_daemon_status() -> Result<()> {
    // Check if daemon is running
    match daemon::is_running()? {
        Some(pid) => {
            println!("Daemon: running (PID {})", pid);
        }
        None => {
            println!("Daemon: not running");
        }
    }

    // Show all watched repos
    let conn = db::open()?;
    let watched = db::list_watched_repos(&conn)?;

    if watched.is_empty() {
        println!("\nNo repos being watched.");
        println!("Run `isq link github` in a git repo to add it.");
    } else {
        println!("\nWatched repos:");
        for watched_repo in &watched {
            // Look up the link to get forge info
            let link = db::get_repo_link(&conn, &watched_repo.repo)?;
            let (forge_repo, forge_type) = match &link {
                Some(l) => (l.forge_repo.clone(), l.forge_type.clone()),
                None => (watched_repo.repo.clone(), "unknown".to_string()),
            };

            let sync_state = db::get_sync_state(&conn, &forge_repo)?;
            let pending = db::count_pending_ops(&conn, &forge_repo)?;

            let sync_info = match sync_state {
                Some((last_sync, count)) => format!("{} issues ({})", count, last_sync),
                None => "not synced".to_string(),
            };

            let pending_info = if pending > 0 {
                format!(" [{} pending]", pending)
            } else {
                String::new()
            };

            println!("  {} [{}]", forge_repo, forge_type);
            println!("    {}{}", sync_info, pending_info);
        }
    }

    Ok(())
}

fn cmd_daemon_start() -> Result<()> {
    daemon::spawn()
}

fn cmd_daemon_stop() -> Result<()> {
    daemon::stop()
}

fn cmd_daemon_watch() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    // Check if repo is linked
    let link = db::get_repo_link(&conn, &repo_path)?
        .ok_or_else(|| anyhow::anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n  isq link github\n  isq link linear"))?;

    db::add_watched_repo(&conn, &repo_path)?;
    println!("✓ Watching {} ({})", link.forge_repo, repo_path);
    Ok(())
}

fn cmd_daemon_unwatch() -> Result<()> {
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;
    db::remove_watched_repo(&conn, &repo_path)?;
    println!("✓ Stopped watching {}", repo_path);
    Ok(())
}

fn print_issues(issues: &[Issue]) {
    if issues.is_empty() {
        println!("No open issues.");
        return;
    }

    for issue in issues {
        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        let labels_str = if labels.is_empty() {
            String::new()
        } else {
            format!("  [{}]", labels.join(", "))
        };

        println!("#{:<6} {}{}", issue.number, issue.title, labels_str);
    }
}

fn print_issue_detail(issue: &Issue) {
    let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();

    println!("#{} {}", issue.number, issue.title);
    println!("State: {}", issue.state);
    println!("Author: {}", issue.user.login);
    if !labels.is_empty() {
        println!("Labels: {}", labels.join(", "));
    }
    println!("Created: {}", issue.created_at);
    println!("Updated: {}", issue.updated_at);

    if let Some(body) = &issue.body {
        println!("\n{}", body);
    }
}
