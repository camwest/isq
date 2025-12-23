mod auth;
mod daemon;
mod db;
mod display;
mod forges;
mod oauth;
mod repo;
mod service;

use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::forges::{get_forge_for_repo, CreateGoalRequest, CreateIssueRequest, Forge, ForgeType, GitHubClient, Issue, LinearClient};

/// JSON response for write operations
#[derive(Serialize)]
struct WriteResult {
    success: bool,
    queued: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_number: Option<u64>,
    message: String,
    elapsed_ms: u64,
}

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

    /// Goal operations (milestones/projects)
    Goal {
        #[command(subcommand)]
        command: GoalCommands,
    },
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

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Add a comment to an issue
    Comment {
        /// Issue number
        id: u64,

        /// Comment body
        message: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Close an issue
    Close {
        /// Issue number
        id: u64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Reopen an issue
    Reopen {
        /// Issue number
        id: u64,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage labels on an issue
    Label {
        /// Issue number
        id: u64,

        /// Action: add or remove
        action: String,

        /// Label name
        label: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Assign a user to an issue
    Assign {
        /// Issue number
        id: u64,

        /// Username to assign
        user: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum GoalCommands {
    /// List goals
    List {
        /// Filter by state (open, closed, all)
        #[arg(long, default_value = "open")]
        state: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show a goal with its issues
    Show {
        /// Goal name or ID
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new goal
    Create {
        /// Goal name
        name: String,

        /// Target date (YYYY-MM-DD)
        #[arg(long)]
        target: Option<String>,

        /// Description
        #[arg(long)]
        body: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Assign an issue to a goal
    Assign {
        /// Issue number
        issue: u64,

        /// Goal name or ID
        goal: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Close a goal
    Close {
        /// Goal name or ID
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
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
            IssueCommands::Create { title, body, label, json } => {
                cmd_issue_create(title, body, label, json).await?
            }
            IssueCommands::Comment { id, message, json } => cmd_issue_comment(id, message, json).await?,
            IssueCommands::Close { id, json } => cmd_issue_close(id, json).await?,
            IssueCommands::Reopen { id, json } => cmd_issue_reopen(id, json).await?,
            IssueCommands::Label { id, action, label, json } => {
                cmd_issue_label(id, action, label, json).await?
            }
            IssueCommands::Assign { id, user, json } => cmd_issue_assign(id, user, json).await?,
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
        Commands::Goal { command } => match command {
            GoalCommands::List { state, json } => cmd_goal_list(state, json).await?,
            GoalCommands::Show { name, json } => cmd_goal_show(name, json)?,
            GoalCommands::Create { name, target, body, json } => {
                cmd_goal_create(name, target, body, json).await?
            }
            GoalCommands::Assign { issue, goal, json } => {
                cmd_goal_assign(issue, goal, json).await?
            }
            GoalCommands::Close { name, json } => cmd_goal_close(name, json).await?,
        },
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
            let conn = db::open()?;

            // Try existing auth first, fall back to OAuth
            let (token, auth_method) = match auth::get_github_token() {
                Ok(t) => (t, if auth::get_gh_token().is_ok() { "gh CLI" } else { "stored" }),
                Err(_) => {
                    // No existing auth - run OAuth flow
                    let token_response = oauth::github_oauth_flow().await?;

                    // Store the token
                    db::set_credential(
                        &conn,
                        "github",
                        &token_response.access_token,
                        token_response.refresh_token.as_deref(),
                        None, // GitHub tokens don't expire by default
                    )?;

                    (token_response.access_token, "OAuth")
                }
            };

            let forge = GitHubClient::new(token);

            // Verify authentication
            let username = forge.get_user().await?;
            println!("✓ Authenticated as {} (via {})", username, auth_method);

            // Save the link
            let display_name = repo.full_name();
            db::set_repo_link(&conn, &repo_path, "github", &repo.full_name(), Some(&display_name))?;

            // Do initial sync
            println!("Syncing {}...", repo.full_name());
            let issues = forge.list_issues(&repo).await?;
            db::save_issues(&conn, &repo.full_name(), &issues)?;

            // Add to watch list (using repo_path as key)
            db::add_watched_repo(&conn, &repo_path)?;

            println!("✓ Cached {} open issues", issues.len());

            // Install and start service
            println!();
            ensure_service_running()?;

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

            let client = LinearClient::new(token_response.access_token.clone());

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

            // Get organization info for display name
            let org = client.get_organization().await?;
            let display_name = format!("{}/{}", org.url_key, team.key);

            // Save the link (use team_id as forge_repo, formatted as "team-key/team-id")
            let forge_repo = format!("{}/{}", team.key, team.id);
            db::set_repo_link(&conn, &repo_path, "linear", &forge_repo, Some(&display_name))?;

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

            // Install and start service
            println!();
            ensure_service_running()?;

            println!("\n✓ Linked to Linear ({})", team.name);
        }
    }

    Ok(())
}

/// Ensure the system service is installed and running
fn ensure_service_running() -> Result<()> {
    let status = service::status()?;

    if !status.installed {
        println!("Installing system service...");
        service::install()?;
        println!("✓ System service installed");
    } else if !status.running {
        service::start()?;
        println!("✓ System service started");
    } else if let Some(pid) = status.pid {
        println!("System service running (PID {})", pid);
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

    // Check if any repos left - if not, uninstall service
    let remaining = db::list_watched_repos(&conn)?;
    if remaining.is_empty() {
        println!();
        service::uninstall()?;
        println!("✓ System service removed (no repos to watch)");
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    // Auth status
    println!("Authentication:");

    // GitHub
    print!("  GitHub    ");
    if auth::get_gh_token().is_ok() {
        println!("ready (via gh CLI)");
    } else if let Ok(conn) = db::open() {
        if db::get_credential(&conn, "github")?.is_some() {
            println!("ready (via OAuth)");
        } else if std::env::var("GITHUB_TOKEN").is_ok() {
            println!("ready (via GITHUB_TOKEN)");
        } else {
            println!("not configured (run: isq link github)");
        }
    } else {
        println!("not configured (run: isq link github)");
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
                    let display = link.display_name.as_deref().unwrap_or(&link.forge_repo);
                    println!("This repo:");
                    println!("  Linked to {} ({})", display, link.forge_type);

                    // Show sync state
                    if let Some((last_sync, count)) = db::get_sync_state(&conn, &link.forge_repo)? {
                        println!("  {} issues cached ({})", count, last_sync);
                    }

                    // Show pending ops
                    let pending = db::count_pending_ops(&conn, &link.forge_repo)?;
                    if pending > 0 {
                        println!("  {} pending operations", pending);
                    }

                    // Show rate limit status
                    if let Some(state) = db::get_rate_limit_state(&conn, &link.forge_type)? {
                        if let Some(reset_at) = state.reset_at {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs() as i64;
                            if now < reset_at {
                                let wait_secs = reset_at - now;
                                // Convert to local time, 12-hour format like macOS default
                                let reset_time = chrono::DateTime::from_timestamp(reset_at, 0)
                                    .map(|dt| {
                                        use chrono::Local;
                                        let local: chrono::DateTime<Local> = dt.into();
                                        local.format("%-I:%M %p").to_string()
                                    })
                                    .unwrap_or_else(|| format!("{}s", wait_secs));
                                println!("  ⚠️  Rate limited until {}", reset_time);
                            }
                        }
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

    // Service status
    println!();
    print!("Service:    ");
    let svc_status = service::status()?;
    if !svc_status.installed {
        println!("not installed");
    } else if let Some(pid) = svc_status.pid {
        println!("running (PID {})", pid);
    } else {
        println!("installed but not running");
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
    let comments = forge.list_all_comments(&repo).await?;
    let goals = forge.list_goals(&repo).await?;
    let fetch_time = start.elapsed();

    let conn = db::open()?;
    db::save_issues(&conn, &link.forge_repo, &issues)?;
    db::save_comments(&conn, &link.forge_repo, &comments)?;
    db::save_goals(&conn, &link.forge_repo, &goals)?;

    // Touch repo to update last_accessed
    db::touch_repo(&conn, &repo_path)?;

    println!(
        "✓ Synced {} issues, {} comments, and {} goals in {:.2}s",
        issues.len(),
        comments.len(),
        goals.len(),
        fetch_time.as_secs_f64()
    );

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
    let comment_counts = db::count_comments_by_issue(&conn, &link.forge_repo)?;
    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        print_issues(&issues, &comment_counts);
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
    let comments = db::load_comments(&conn, &link.forge_repo, id)?;
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
                // Use styled display
                display::print_issue(&issue, &comments, elapsed.as_millis() as u64);
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

async fn cmd_issue_create(title: String, body: Option<String>, labels: Vec<String>, json: bool) -> Result<()> {
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
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: Some(issue.number),
                    message: format!("Created #{} {}", issue.number, issue.title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Created #{} {} ({:.0}ms)",
                    issue.number, issue.title, elapsed.as_millis()
                );
            }
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
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: None,
                    message: format!("Queued: {}", title),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Queued: {} (offline, {:.0}ms)",
                    title, elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_comment(id: u64, message: String, json: bool) -> Result<()> {
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
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: Some(id),
                    message: format!("Comment added to #{}", id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Comment added to #{} ({:.0}ms)", id, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_number": id,
                "body": message,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "comment", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: Some(id),
                    message: format!("Queued: comment on #{}", id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Queued: comment on #{} (offline, {:.0}ms)",
                    id, elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_close(id: u64, json: bool) -> Result<()> {
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
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: Some(id),
                    message: format!("Closed #{}", id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Closed #{} ({:.0}ms)", id, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_number": id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "close", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: Some(id),
                    message: format!("Queued: close #{}", id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Queued: close #{} (offline, {:.0}ms)", id, elapsed.as_millis());
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_reopen(id: u64, json: bool) -> Result<()> {
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
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: Some(id),
                    message: format!("Reopened #{}", id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Reopened #{} ({:.0}ms)", id, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({ "issue_number": id });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "reopen", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: Some(id),
                    message: format!("Queued: reopen #{}", id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Queued: reopen #{} (offline, {:.0}ms)", id, elapsed.as_millis());
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_issue_label(id: u64, action: String, label: String, json: bool) -> Result<()> {
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
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: false,
                            issue_number: Some(id),
                            message: format!("Added label '{}' to #{}", label, id),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!("✓ Added label '{}' to #{} ({:.0}ms)", label, id, elapsed.as_millis());
                    }
                }
                Err(e) if is_offline_error(&e) => {
                    let elapsed = start.elapsed();
                    let payload = serde_json::json!({
                        "issue_number": id,
                        "label": label,
                    });
                    let conn = db::open()?;
                    db::queue_op(&conn, &link.forge_repo, "label_add", &payload.to_string())?;
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: true,
                            issue_number: Some(id),
                            message: format!("Queued: add label '{}' to #{}", label, id),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!(
                            "✓ Queued: add label '{}' to #{} (offline, {:.0}ms)",
                            label, id, elapsed.as_millis()
                        );
                    }
                }
                Err(e) => return Err(e),
            }
        }
        "remove" => {
            match forge.remove_label(&repo, id, &label).await {
                Ok(()) => {
                    let elapsed = start.elapsed();
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: false,
                            issue_number: Some(id),
                            message: format!("Removed label '{}' from #{}", label, id),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!("✓ Removed label '{}' from #{} ({:.0}ms)", label, id, elapsed.as_millis());
                    }
                }
                Err(e) if is_offline_error(&e) => {
                    let elapsed = start.elapsed();
                    let payload = serde_json::json!({
                        "issue_number": id,
                        "label": label,
                    });
                    let conn = db::open()?;
                    db::queue_op(&conn, &link.forge_repo, "label_remove", &payload.to_string())?;
                    if json {
                        let result = WriteResult {
                            success: true,
                            queued: true,
                            issue_number: Some(id),
                            message: format!("Queued: remove label '{}' from #{}", label, id),
                            elapsed_ms: elapsed.as_millis() as u64,
                        };
                        println!("{}", serde_json::to_string_pretty(&result)?);
                    } else {
                        println!(
                            "✓ Queued: remove label '{}' from #{} (offline, {:.0}ms)",
                            label, id, elapsed.as_millis()
                        );
                    }
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

async fn cmd_issue_assign(id: u64, user: String, json: bool) -> Result<()> {
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
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: Some(id),
                    message: format!("Assigned @{} to #{}", user, id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Assigned @{} to #{} ({:.0}ms)", user, id, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_number": id,
                "assignee": user,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "assign", &payload.to_string())?;
            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: Some(id),
                    message: format!("Queued: assign @{} to #{}", user, id),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "✓ Queued: assign @{} to #{} (offline, {:.0}ms)",
                    user, id, elapsed.as_millis()
                );
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

fn cmd_daemon_status() -> Result<()> {
    // Check service status
    let status = service::status()?;

    if !status.installed {
        println!("Service: not installed");
        println!("         Run `isq link <forge>` to install");
    } else if !status.running {
        println!("Service: installed but not running");
    } else if let Some(pid) = status.pid {
        println!("Service: running (PID {})", pid);
    } else {
        println!("Service: running");
    }

    // Clean up stale repo entries before displaying
    let conn = db::open()?;
    let removed = db::cleanup_stale_repos(&conn)?;
    if removed > 0 {
        println!("\n(Cleaned up {} stale entries)", removed);
    }

    // Show all watched sources
    let watched = db::list_watched_repos(&conn)?;

    if watched.is_empty() {
        println!("\nNothing being watched.");
        println!("Run `isq link github` or `isq link linear` in a git repo to add it.");
    } else {
        println!("\nWatching:");
        for watched_repo in &watched {
            // Look up the link to get forge info
            let link = db::get_repo_link(&conn, &watched_repo.repo)?;
            let (display, forge_repo, forge_type) = match &link {
                Some(l) => {
                    // Use display_name if available, fall back to forge_repo
                    let display = l.display_name.clone().unwrap_or_else(|| l.forge_repo.clone());
                    (display, l.forge_repo.clone(), l.forge_type.clone())
                }
                None => (watched_repo.repo.clone(), watched_repo.repo.clone(), "unknown".to_string()),
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

            println!("  {} [{}]", display, forge_type);
            println!("    {}{}", sync_info, pending_info);
        }
    }

    Ok(())
}

fn cmd_daemon_start() -> Result<()> {
    service::start()?;
    println!("✓ Service started");
    Ok(())
}

fn cmd_daemon_stop() -> Result<()> {
    service::stop()?;
    println!("✓ Service stopped");
    Ok(())
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

fn print_issues(issues: &[Issue], comment_counts: &std::collections::HashMap<u64, usize>) {
    if issues.is_empty() {
        println!("No open issues.");
        return;
    }

    for issue in issues {
        let count = comment_counts.get(&issue.number).copied();
        display::print_issue_row(issue, count);
    }
}

// ============================================================================
// Goal Commands
// ============================================================================

async fn cmd_goal_list(state: String, json_output: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    let link = db::get_repo_link(&conn, &repo_path)?
        .ok_or_else(|| anyhow::anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n  isq link github\n  isq link linear"))?;

    // Load goals from cache, filtering by state if not "all"
    let state_filter = if state == "all" { None } else { Some(state.as_str()) };
    let mut goals = db::load_goals(&conn, &link.forge_repo, state_filter)?;

    // If no cached goals, fetch from API
    if goals.is_empty() && db::count_goals(&conn, &link.forge_repo)? == 0 {
        eprintln!("Syncing goals...");
        let (forge, _) = get_forge_for_repo(&repo_path)?;

        // Parse forge_repo to create Repo struct
        let parts: Vec<&str> = link.forge_repo.split('/').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
        }
        let repo = repo::Repo {
            owner: parts[0].to_string(),
            name: parts[1].to_string(),
        };

        let fetched = forge.list_goals(&repo).await?;
        db::save_goals(&conn, &link.forge_repo, &fetched)?;

        // Re-filter after saving
        goals = db::load_goals(&conn, &link.forge_repo, state_filter)?;
    }

    db::touch_repo(&conn, &repo_path)?;
    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&goals)?);
    } else {
        display::print_goals(&goals);
        eprintln!("\n{} goals in {:.0}ms", goals.len(), elapsed.as_millis());
    }

    Ok(())
}

fn cmd_goal_show(name: String, json_output: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let conn = db::open()?;

    let link = db::get_repo_link(&conn, &repo_path)?
        .ok_or_else(|| anyhow::anyhow!("This repo is not linked to an issue tracker.\n\nRun one of:\n  isq link github\n  isq link linear"))?;

    db::touch_repo(&conn, &repo_path)?;

    let goal = db::load_goal_by_name(&conn, &link.forge_repo, &name)?
        .ok_or_else(|| anyhow::anyhow!("Goal '{}' not found. Run `isq sync` to refresh.", name))?;

    let elapsed = start.elapsed();

    if json_output {
        println!("{}", serde_json::to_string_pretty(&goal)?);
    } else {
        display::print_goal_detail(&goal, elapsed.as_millis() as u64);
    }

    Ok(())
}

async fn cmd_goal_create(name: String, target: Option<String>, body: Option<String>, json: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    let req = CreateGoalRequest {
        name: name.clone(),
        description: body.clone(),
        target_date: target.clone(),
    };

    match forge.create_goal(&repo, req).await {
        Ok(goal) => {
            let elapsed = start.elapsed();
            // Save to local cache
            let conn = db::open()?;
            db::save_goal(&conn, &link.forge_repo, &goal)?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: None,
                    message: format!("Created goal: {}", goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Created goal: {} ({:.0}ms)", goal.name, elapsed.as_millis());
                if let Some(url) = &goal.html_url {
                    println!("  {}", url);
                }
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "name": name,
                "target_date": target,
                "description": body,
            });
            let conn = db::open()?;
            db::queue_op(&conn, &link.forge_repo, "create_goal", &payload.to_string())?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: None,
                    message: format!("Queued: create goal {}", name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Queued: create goal {} (offline, {:.0}ms)", name, elapsed.as_millis());
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_goal_assign(issue: u64, goal_name: String, json: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;
    let conn = db::open()?;

    // Resolve goal name to ID
    let goal = db::load_goal_by_name(&conn, &link.forge_repo, &goal_name)?
        .ok_or_else(|| anyhow::anyhow!("Goal '{}' not found. Run `isq sync` to refresh.", goal_name))?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.assign_to_goal(&repo, issue, &goal.id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: Some(issue),
                    message: format!("Assigned #{} to goal '{}'", issue, goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Assigned #{} to goal '{}' ({:.0}ms)", issue, goal.name, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "issue_number": issue,
                "goal_id": goal.id,
            });
            db::queue_op(&conn, &link.forge_repo, "assign_goal", &payload.to_string())?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: Some(issue),
                    message: format!("Queued: assign #{} to '{}'", issue, goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Queued: assign #{} to '{}' (offline, {:.0}ms)", issue, goal.name, elapsed.as_millis());
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

async fn cmd_goal_close(name: String, json: bool) -> Result<()> {
    let start = Instant::now();
    let repo_path = repo::detect_repo_path()?;
    let (forge, link) = get_forge_for_repo(&repo_path)?;
    let conn = db::open()?;

    // Resolve goal name to ID
    let goal = db::load_goal_by_name(&conn, &link.forge_repo, &name)?
        .ok_or_else(|| anyhow::anyhow!("Goal '{}' not found. Run `isq sync` to refresh.", name))?;

    let parts: Vec<&str> = link.forge_repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid forge_repo format: {}", link.forge_repo);
    }
    let repo = repo::Repo {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
    };

    match forge.close_goal(&repo, &goal.id).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            if json {
                let result = WriteResult {
                    success: true,
                    queued: false,
                    issue_number: None,
                    message: format!("Closed goal '{}'", goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Closed goal '{}' ({:.0}ms)", goal.name, elapsed.as_millis());
            }
        }
        Err(e) if is_offline_error(&e) => {
            let elapsed = start.elapsed();
            let payload = serde_json::json!({
                "goal_id": goal.id,
            });
            db::queue_op(&conn, &link.forge_repo, "close_goal", &payload.to_string())?;

            if json {
                let result = WriteResult {
                    success: true,
                    queued: true,
                    issue_number: None,
                    message: format!("Queued: close goal '{}'", goal.name),
                    elapsed_ms: elapsed.as_millis() as u64,
                };
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("✓ Queued: close goal '{}' (offline, {:.0}ms)", goal.name, elapsed.as_millis());
            }
        }
        Err(e) => return Err(e),
    }

    Ok(())
}

