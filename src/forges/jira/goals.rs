//! JIRA goal (version) operations

use anyhow::Result;

use super::client::JiraClient;
use super::types::JiraVersion;
use crate::forges::{Goal, GoalState};
use crate::repo::Repo;

impl JiraClient {
    /// List goals (versions) for a project
    pub async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        let project_key = &repo.name;
        let path = format!("/project/{}/versions", project_key);

        let versions: Vec<JiraVersion> = self.get(&path).await?;

        let goals: Vec<Goal> = versions
            .into_iter()
            .map(|v| {
                let state = if v.released.unwrap_or(false) || v.archived.unwrap_or(false) {
                    GoalState::Closed
                } else {
                    GoalState::Open
                };

                Goal {
                    id: v.id,
                    name: v.name,
                    description: v.description,
                    target_date: v.release_date,
                    state,
                    progress: 0.0, // TODO: calculate from issues
                    open_count: None,
                    closed_count: None,
                    created_at: String::new(), // Versions don't have created_at
                    updated_at: String::new(),
                    html_url: None,
                }
            })
            .collect();

        Ok(goals)
    }
}
