mod auth;
mod github;
mod repo;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::github::GitHubClient;

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
}

#[derive(Subcommand)]
enum DaemonCommands {
    /// Show daemon status
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Auth => cmd_auth()?,
        Commands::Issue { command } => match command {
            IssueCommands::List { json } => cmd_issue_list(json).await?,
            IssueCommands::Show { id, json } => cmd_issue_show(id, json).await?,
        },
        Commands::Daemon { command } => match command {
            DaemonCommands::Status => {
                println!("isq daemon status: not implemented");
            }
        },
        Commands::Sync => {
            println!("isq sync: not implemented");
        }
    }

    Ok(())
}

fn cmd_auth() -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;

    // Mask the token for display
    let masked = format!("{}...{}", &token[..4], &token[token.len() - 4..]);

    println!("Found existing gh CLI authentication.");
    println!("✓ Logged in (token: {})", masked);
    println!("✓ Detected repo: {}", repo.full_name());

    Ok(())
}

async fn cmd_issue_list(json_output: bool) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    eprintln!("Fetching issues from {}...", repo.full_name());

    let issues = client.list_issues(&repo).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        if issues.is_empty() {
            println!("No open issues.");
        } else {
            for issue in &issues {
                let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
                let labels_str = if labels.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", labels.join(", "))
                };

                println!("#{:<6} {}{}", issue.number, issue.title, labels_str);
            }
            eprintln!("\n{} open issues", issues.len());
        }
    }

    Ok(())
}

async fn cmd_issue_show(id: u64, json_output: bool) -> Result<()> {
    let token = auth::get_gh_token()?;
    let repo = repo::detect_repo()?;
    let client = GitHubClient::new(token);

    let issue = client.get_issue(&repo, id).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&issue)?);
    } else {
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

    Ok(())
}
