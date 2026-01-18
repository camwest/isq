//! CLI module - command routing and handlers
//!
//! This module implements the command-line interface using clap,
//! with handlers organized by domain (issues, goals, daemon, etc.)

mod args;
pub mod daemon;
pub mod doctor;
pub mod forge;
pub mod goals;
pub mod install;
pub mod issues;
pub mod labels;
pub mod link;
pub mod status;
pub mod sync;
pub mod uninstall;
pub mod update;
mod utils;
mod version;
pub mod views;
pub mod worktree;

pub use args::{
    Cli, Commands, DaemonCommands, GoalCommands, InstallCommands, IssueCommands, LabelCommands,
    UpdateCommands, ViewCommands,
};
pub use version::print_version;
