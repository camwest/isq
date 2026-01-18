//! Tests for issue CRUD operations

mod crud;
mod duration;
mod filters;
mod issue_filter;
mod labels;
mod sorting;

use crate::db::schema::init_schema;
use crate::forges::{Issue, Label};
use rusqlite::Connection;

/// Create an in-memory database for testing
pub fn test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    conn
}

pub fn make_issue(id: &str, title: &str, state: &str, labels: Vec<&str>) -> Issue {
    Issue {
        id: id.to_string(),
        title: title.to_string(),
        body: None,
        state: state.to_string(),
        author: "testuser".to_string(),
        labels: labels
            .into_iter()
            .map(|s| Label::name_only(s.to_string()))
            .collect(),
        assignees: vec![],
        priority: 4, // Default: none
        priority_label: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
        url: None,
        milestone: None,
    }
}
