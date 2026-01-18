//! Linear user operations

use anyhow::Result;

use super::LinearClient;
use super::types::{IssueUpdateResponse, LinearUserWithId, UsersResponse};

impl LinearClient {
    /// Get user by name or email
    pub async fn get_user_by_name(&self, name: &str) -> Result<LinearUserWithId> {
        let query = r#"
            query {
                users {
                    nodes {
                        id
                        name
                        email
                    }
                }
            }
        "#;

        let response: UsersResponse = self.query(query, None).await?;

        // Try to match by name (case-insensitive) or email
        let name_lower = name.to_lowercase();
        response
            .users
            .nodes
            .into_iter()
            .find(|u| u.name.to_lowercase() == name_lower || u.email.to_lowercase() == name_lower)
            .ok_or_else(|| anyhow::anyhow!("User '{}' not found", name))
    }

    /// Assign issue by user ID directly (no name lookup)
    pub async fn assign_issue_by_id(
        &self,
        team_id: &str,
        issue_number: u64,
        user_id: &str,
    ) -> Result<()> {
        let issue = self.get_issue_by_number(team_id, issue_number).await?;

        let query = r#"
            mutation($issueId: String!, $assigneeId: String!) {
                issueUpdate(id: $issueId, input: { assigneeId: $assigneeId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "assigneeId": user_id
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to assign issue");
        }
        Ok(())
    }
}
