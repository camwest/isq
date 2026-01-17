//! CLI module - command routing and handlers
//!
//! This module implements the command-line interface using clap,
//! with handlers organized by domain (issues, goals, daemon, etc.)

mod args;
pub mod daemon;
pub mod forge;
pub mod goals;
pub mod install;
pub mod issues;
pub mod labels;
pub mod link;
pub mod status;
pub mod sync;
mod utils;
pub mod views;
pub mod worktree;

pub use args::{Cli, Commands, DaemonCommands, GoalCommands, InstallCommands, IssueCommands, LabelCommands, ViewCommands};
