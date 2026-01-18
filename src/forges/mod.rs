//! Forge abstraction for issue trackers (GitHub, Linear, JIRA).

mod auth;
mod forge_type;
mod github;
mod jira;
mod linear;
mod link;
mod traits;
mod types;

#[cfg(test)]
mod tests;

// Re-export public items for external use
pub use auth::AuthConfig;
pub use forge_type::{ALL_FORGE_TYPES, ForgeType, get_forge_for_repo};
pub use github::GitHubClient;
pub use jira::JiraClient;
pub use linear::LinearClient;
pub use link::{LinkArgs, LinkResult, not_linked_error};
pub use traits::Forge;
pub use types::{
    CreateGoalRequest, CreateIssueRequest, FetchResult, Goal, GoalState, Issue, Label,
    RateLimitInfo, create_http_client, parse_opts,
};
