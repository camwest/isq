//! Linear issue update helpers.

use anyhow::Result;

use super::LinearClient;
use super::map_to_linear_priority;
use super::types;
use crate::forges::UpdateIssueRequest;

impl LinearClient {
    /// Update mutable issue fields by Linear internal issue ID.
    pub async fn update_issue_fields_by_id(
        &self,
        issue_id: &str,
        req: UpdateIssueRequest,
    ) -> Result<()> {
        let mut input = serde_json::Map::new();

        if let Some(title) = req.title {
            input.insert("title".to_string(), serde_json::json!(title));
        }
        if let Some(body) = req.body {
            input.insert("description".to_string(), serde_json::json!(body));
        }
        if let Some(priority) = req.priority {
            input.insert(
                "priority".to_string(),
                serde_json::json!(map_to_linear_priority(priority)),
            );
        }

        if input.is_empty() {
            anyhow::bail!("No fields provided to update");
        }

        let query = r#"
            mutation($issueId: String!, $input: IssueUpdateInput!) {
                issueUpdate(id: $issueId, input: $input) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue_id,
            "input": input
        });

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to update issue");
        }
        Ok(())
    }
}
