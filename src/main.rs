mod auth;
mod daemon;
mod db;
mod forge;
mod github;
mod repo;

use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::forge::{CreateIssueRequest, Forge};
use crate::github::{GitHubClient, Issue};

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
    /// Authenticate with your forge
    Auth,

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
    /// Show daemon status
    Status,

    /// Start the daemon
    Start,

    /// Stop the daemon
    Stop,

    /// Run the sync loop (internal, called by spawn)
    #[command(hide = true)]
    Run {
        #[arg(long)]
        repo: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Auth => cmd_auth().await?,
        Commands::Issue { command } => match command {
            IssueCommands::List { label, state, json } => cmd_issue_list(label, state, json)?,
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
            DaemonCommands::Run { repo } => daemon::run_loop(&repo).await?,
        },
        Commands::Sync => cmd_sync().await?,
    }

    Ok(())
}

async fn cmd_auth() -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    // Get username
    let username = client.get_user().await?;

    println!("Found existing gh CLI authentication.");
    println!("✓ Logged in as {} (via gh CLI)", username);
    println!("✓ Detected repo: {}", repo.full_name());

    // Do initial sync
    println!("\nSyncing {}...", repo.full_name());
    let issues = client.list_issues(&repo).await?;

    let conn = db::open()?;
    db::save_issues(&conn, &repo.full_name(), &issues)?;

    println!("✓ Cached {} open issues", issues.len());

    // Start daemon
    println!();
    daemon::spawn(&repo)?;

    Ok(())
}

async fn cmd_sync() -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    eprintln!("Syncing {}...", repo.full_name());
    let start = Instant::now();

    let issues = client.list_issues(&repo).await?;
    let fetch_time = start.elapsed();

    let conn = db::open()?;
    db::save_issues(&conn, &repo.full_name(), &issues)?;

    println!("✓ Synced {} issues in {:.2}s", issues.len(), fetch_time.as_secs_f64());

    Ok(())
}

fn cmd_issue_list(
    label: Option<String>,
    state: Option<String>,
    json_output: bool,
) -> Result<()> {
    let start = Instant::now();

    let repo = repo::detect_repo()?;
    let conn = db::open()?;

    // Check if we have cached data
    let sync_state = db::get_sync_state(&conn, &repo.full_name())?;
    if sync_state.is_none() {
        anyhow::bail!(
            "No cached data for {}. Run `isq auth` or `isq sync` first.",
            repo.full_name()
        );
    }

    let issues = db::load_issues_filtered(
        &conn,
        &repo.full_name(),
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

    let repo = repo::detect_repo()?;
    let conn = db::open()?;

    let issue = db::load_issue(&conn, &repo.full_name(), id)?;
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
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    let req = CreateIssueRequest {
        title,
        body,
        labels,
    };

    let issue = client.create_issue(&repo, req).await?;
    println!("✓ Created #{} {}", issue.number, issue.title);

    Ok(())
}

async fn cmd_issue_comment(id: u64, message: String) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    client.create_comment(&repo, id, &message).await?;
    println!("✓ Comment added to #{}", id);

    Ok(())
}

async fn cmd_issue_close(id: u64) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    client.close_issue(&repo, id).await?;
    println!("✓ Closed #{}", id);

    Ok(())
}

async fn cmd_issue_reopen(id: u64) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    client.reopen_issue(&repo, id).await?;
    println!("✓ Reopened #{}", id);

    Ok(())
}

async fn cmd_issue_label(id: u64, action: String, label: String) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    match action.as_str() {
        "add" => {
            client.add_label(&repo, id, &label).await?;
            println!("✓ Added label '{}' to #{}", label, id);
        }
        "remove" => {
            client.remove_label(&repo, id, &label).await?;
            println!("✓ Removed label '{}' from #{}", label, id);
        }
        _ => {
            anyhow::bail!("Invalid action '{}'. Use 'add' or 'remove'.", action);
        }
    }

    Ok(())
}

async fn cmd_issue_assign(id: u64, user: String) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    client.assign_issue(&repo, id, &user).await?;
    println!("✓ Assigned @{} to #{}", user, id);

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

    // Show sync state for current repo if available
    if let Ok(repo) = repo::detect_repo() {
        let conn = db::open()?;
        match db::get_sync_state(&conn, &repo.full_name())? {
            Some((last_sync, count)) => {
                println!("\n{}: {} issues", repo.full_name(), count);
                println!("Last synced: {}", last_sync);
            }
            None => {
                println!("\n{}: not synced", repo.full_name());
            }
        }
    }

    Ok(())
}

fn cmd_daemon_start() -> Result<()> {
    let repo = repo::detect_repo()?;
    daemon::spawn(&repo)
}

fn cmd_daemon_stop() -> Result<()> {
    daemon::stop()
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
