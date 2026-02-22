//! Tests for repository management

use super::*;
use crate::db::schema::init_schema;
use rusqlite::Connection;

/// Create an in-memory database for testing
fn test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn
}

// === Watched Repos Tests ===

#[test]
fn test_add_watched_repo() {
    let conn = test_db();

    add_watched_repo(&conn, "owner/repo").unwrap();

    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].repo, "owner/repo");
}

#[test]
fn test_add_watched_repo_is_idempotent() {
    let conn = test_db();

    add_watched_repo(&conn, "owner/repo").unwrap();
    add_watched_repo(&conn, "owner/repo").unwrap();

    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos.len(), 1);
}

#[test]
fn test_touch_repo_adds_if_not_exists() {
    let conn = test_db();

    touch_repo(&conn, "owner/repo").unwrap();

    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].repo, "owner/repo");
}

#[test]
fn test_touch_repo_updates_ordering() {
    let conn = test_db();

    // Add two repos
    add_watched_repo(&conn, "first/repo").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    add_watched_repo(&conn, "second/repo").unwrap();

    // Second repo is most recent
    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos[0].repo, "second/repo");

    // Touch first repo to make it most recent
    std::thread::sleep(std::time::Duration::from_millis(1100));
    touch_repo(&conn, "first/repo").unwrap();

    // First repo is now most recent
    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos[0].repo, "first/repo");
}

#[test]
fn test_list_watched_repos_ordered_by_last_accessed() {
    let conn = test_db();

    add_watched_repo(&conn, "old/repo").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    add_watched_repo(&conn, "new/repo").unwrap();

    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos.len(), 2);
    // Most recently accessed first
    assert_eq!(repos[0].repo, "new/repo");
    assert_eq!(repos[1].repo, "old/repo");
}

#[test]
fn test_remove_watched_repo() {
    let conn = test_db();

    add_watched_repo(&conn, "owner/repo").unwrap();
    add_watched_repo(&conn, "other/repo").unwrap();

    remove_watched_repo(&conn, "owner/repo").unwrap();

    let repos = list_watched_repos(&conn).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].repo, "other/repo");
}

#[test]
fn test_remove_watched_repo_nonexistent() {
    let conn = test_db();

    // Should not error
    remove_watched_repo(&conn, "nonexistent/repo").unwrap();
}

// === Repo Links Tests ===

#[test]
fn test_set_and_get_repo_link() {
    let conn = test_db();

    set_repo_link(
        &conn,
        SetRepoLinkParams {
            repo_path: "/path/to/repo",
            forge_type: "github",
            forge_repo: "owner/repo",
            auth_scope: None,
            display_name: None,
            user_id: None,
            user_name: None,
        },
    )
    .unwrap();

    let link = get_repo_link(&conn, "/path/to/repo").unwrap();
    assert!(link.is_some());
    let link = link.unwrap();
    assert_eq!(link.forge_type, "github");
    assert_eq!(link.forge_repo, "owner/repo");
    assert_eq!(link.auth_scope, None);
}

#[test]
fn test_get_repo_link_not_found() {
    let conn = test_db();

    let link = get_repo_link(&conn, "/nonexistent/path").unwrap();
    assert!(link.is_none());
}

#[test]
fn test_set_repo_link_updates_existing() {
    let conn = test_db();

    set_repo_link(
        &conn,
        SetRepoLinkParams {
            repo_path: "/path/to/repo",
            forge_type: "github",
            forge_repo: "owner/repo",
            auth_scope: None,
            display_name: None,
            user_id: None,
            user_name: None,
        },
    )
    .unwrap();
    set_repo_link(
        &conn,
        SetRepoLinkParams {
            repo_path: "/path/to/repo",
            forge_type: "linear",
            forge_repo: "team-id",
            auth_scope: None,
            display_name: None,
            user_id: None,
            user_name: None,
        },
    )
    .unwrap();

    let link = get_repo_link(&conn, "/path/to/repo").unwrap().unwrap();
    assert_eq!(link.forge_type, "linear");
    assert_eq!(link.forge_repo, "team-id");
}

#[test]
fn test_remove_repo_link() {
    let conn = test_db();

    set_repo_link(
        &conn,
        SetRepoLinkParams {
            repo_path: "/path/to/repo",
            forge_type: "github",
            forge_repo: "owner/repo",
            auth_scope: None,
            display_name: None,
            user_id: None,
            user_name: None,
        },
    )
    .unwrap();
    remove_repo_link(&conn, "/path/to/repo").unwrap();

    let link = get_repo_link(&conn, "/path/to/repo").unwrap();
    assert!(link.is_none());
}

#[test]
fn test_remove_repo_link_nonexistent() {
    let conn = test_db();

    // Should not error
    remove_repo_link(&conn, "/nonexistent/path").unwrap();
}

#[test]
fn test_repo_link_with_user_id_and_name() {
    let conn = test_db();

    set_repo_link(
        &conn,
        SetRepoLinkParams {
            repo_path: "/path/to/repo",
            forge_type: "github",
            forge_repo: "owner/repo",
            auth_scope: None,
            display_name: None,
            user_id: Some("user-id-123"),
            user_name: Some("testuser"),
        },
    )
    .unwrap();

    let link = get_repo_link(&conn, "/path/to/repo").unwrap().unwrap();
    assert_eq!(link.user_id, Some("user-id-123".to_string()));
    assert_eq!(link.user_name, Some("testuser".to_string()));
}

#[test]
fn test_repo_link_with_auth_scope() {
    let conn = test_db();

    set_repo_link(
        &conn,
        SetRepoLinkParams {
            repo_path: "/path/to/repo",
            forge_type: "linear",
            forge_repo: "TEAM/1234-uuid",
            auth_scope: Some("linear:acme:viewer-1"),
            display_name: None,
            user_id: None,
            user_name: None,
        },
    )
    .unwrap();

    let link = get_repo_link(&conn, "/path/to/repo").unwrap().unwrap();
    assert_eq!(link.forge_type, "linear");
    assert_eq!(link.auth_scope, Some("linear:acme:viewer-1".to_string()));
}

// === Worktree Issues Tests ===

#[test]
fn test_set_and_get_worktree_issue() {
    let conn = test_db();

    // Initially no issue associated
    let result = get_worktree_issue(&conn, "/path/to/.git").unwrap();
    assert!(result.is_none());

    // Set an issue
    set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "123").unwrap();

    // Get the issue back
    let result = get_worktree_issue(&conn, "/path/to/.git").unwrap();
    assert!(result.is_some());
    let (repo, issue_id) = result.unwrap();
    assert_eq!(repo, "owner/repo");
    assert_eq!(issue_id, "123");
}

#[test]
fn test_worktree_issue_replaces_existing() {
    let conn = test_db();

    // Set first issue
    set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "100").unwrap();

    // Set a different issue (should replace)
    set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "200").unwrap();

    // Should get the new issue
    let result = get_worktree_issue(&conn, "/path/to/.git").unwrap().unwrap();
    assert_eq!(result.1, "200");
}

#[test]
fn test_clear_worktree_issues() {
    let conn = test_db();

    // Set an issue
    set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "123").unwrap();

    // Verify it exists
    assert!(
        get_worktree_issue(&conn, "/path/to/.git")
            .unwrap()
            .is_some()
    );

    // Clear it
    clear_worktree_issues(&conn, "/path/to/.git").unwrap();

    // Verify it's gone
    assert!(
        get_worktree_issue(&conn, "/path/to/.git")
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_worktree_issues_isolated_by_git_dir() {
    let conn = test_db();

    // Set issues for different worktrees
    set_worktree_issue(&conn, "/path/to/.git", "owner/repo", "100").unwrap();
    set_worktree_issue(
        &conn,
        "/path/to/.git/worktrees/feature",
        "owner/repo",
        "DEV-200",
    )
    .unwrap();

    // Each worktree should have its own issue
    let main = get_worktree_issue(&conn, "/path/to/.git").unwrap().unwrap();
    let feature = get_worktree_issue(&conn, "/path/to/.git/worktrees/feature")
        .unwrap()
        .unwrap();

    assert_eq!(main.1, "100");
    assert_eq!(feature.1, "DEV-200");
}
