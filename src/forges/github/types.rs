//! GitHub API response types

use serde::Deserialize;

use crate::forges::{Goal, GoalState, Issue, Label};

/// GitHub API issue response
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub user: GitHubUser,
    pub labels: Vec<GitHubLabel>,
    #[serde(default)]
    pub assignees: Vec<GitHubUser>,
    pub milestone: Option<GitHubMilestoneRef>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub html_url: Option<String>,
    /// Present only if this is actually a PR (GitHub returns PRs in issues endpoint)
    pub pull_request: Option<serde_json::Value>,
}

impl GitHubIssue {
    pub fn into_issue(self) -> Issue {
        Issue {
            id: self.number.to_string(),
            title: self.title,
            body: self.body,
            state: self.state,
            author: self.user.login,
            labels: self
                .labels
                .into_iter()
                .map(|l| Label::new(l.name, Some(l.color)))
                .collect(),
            assignees: self.assignees.into_iter().map(|u| u.login).collect(),
            priority: 4, // Default: none (will be overridden if priority config exists)
            priority_label: None,
            created_at: self.created_at,
            updated_at: self.updated_at,
            url: self.html_url,
            milestone: self.milestone.map(|m| m.title),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubUser {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
}

/// Minimal milestone info embedded in issue responses
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubMilestoneRef {
    pub title: String,
}

/// GitHub API comment response
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubComment {
    pub id: u64,
    pub issue_url: String,
    pub body: String,
    pub user: GitHubUser,
    pub created_at: String,
    pub updated_at: String,
}

impl GitHubComment {
    /// Parse issue ID from issue_url (e.g., "https://api.github.com/repos/owner/repo/issues/123")
    pub fn issue_id(&self) -> Option<String> {
        self.issue_url.rsplit('/').next().map(|s| s.to_string())
    }
}

/// GitHub API milestone response
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubMilestone {
    pub number: u64,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub open_issues: u64,
    pub closed_issues: u64,
    pub due_on: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: String,
}

impl From<GitHubMilestone> for Goal {
    fn from(m: GitHubMilestone) -> Self {
        let total = m.open_issues + m.closed_issues;
        let progress = if total > 0 {
            m.closed_issues as f64 / total as f64
        } else {
            0.0
        };

        Goal {
            id: m.number.to_string(),
            name: m.title,
            description: m.description,
            // Extract YYYY-MM-DD from ISO 8601 datetime
            target_date: m.due_on.map(|d| d.chars().take(10).collect()),
            state: if m.state == "open" {
                GoalState::Open
            } else {
                GoalState::Closed
            },
            progress,
            open_count: Some(m.open_issues),
            closed_count: Some(m.closed_issues),
            created_at: m.created_at,
            updated_at: m.updated_at,
            html_url: Some(m.html_url),
        }
    }
}

#[derive(Deserialize)]
pub struct SearchResult {
    pub total_count: usize,
}
