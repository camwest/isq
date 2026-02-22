//! Repository management - watched repos, repo links, and worktree issues

mod links;
mod watched;
mod worktree;

#[cfg(test)]
mod tests;

// Re-export public items
pub use links::{RepoLink, SetRepoLinkParams, get_repo_link, remove_repo_link, set_repo_link};
pub use watched::{
    WatchedRepo, add_watched_repo, cleanup_stale_repos, list_watched_repos, remove_watched_repo,
    touch_repo,
};
pub use worktree::{
    clear_worktree_issues, get_worktree_issue, get_worktree_issue_ids, set_worktree_issue,
};
