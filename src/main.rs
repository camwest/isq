use anyhow::Result;
use clap::{Parser, Subcommand};

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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Auth => {
            println!("isq auth: not implemented");
        }
        Commands::Issue { command } => match command {
            IssueCommands::List { json } => {
                if json {
                    println!("isq issue list --json: not implemented");
                } else {
                    println!("isq issue list: not implemented");
                }
            }
            IssueCommands::Show { id, json } => {
                if json {
                    println!("isq issue show {} --json: not implemented", id);
                } else {
                    println!("isq issue show {}: not implemented", id);
                }
            }
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
