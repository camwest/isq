//! Linear workflow state operations

use anyhow::Result;

use super::LinearClient;
use super::types::{IssueUpdateResponse, WorkflowState, WorkflowStatesResponse};

impl LinearClient {
    /// Get workflow state by type (completed, started, backlog, etc.)
    pub async fn get_state_by_type(
        &self,
        team_id: &str,
        state_type: &str,
    ) -> Result<WorkflowState> {
        let query = r#"
            query($teamId: ID!) {
                workflowStates(filter: { team: { id: { eq: $teamId } } }) {
                    nodes {
                        id
                        name
                        type
                    }
                }
            }
        "#;

        let variables = serde_json::json!({ "teamId": team_id });
        let response: WorkflowStatesResponse = self.query(query, Some(variables)).await?;

        response
            .workflow_states
            .nodes
            .into_iter()
            .find(|s| s.state_type == state_type)
            .ok_or_else(|| anyhow::anyhow!("No workflow state of type '{}' found", state_type))
    }

    /// Get workflow state by type OR name
    /// Tries matching by type first (stable), then by name (customizable)
    pub async fn get_state_by_type_or_name(
        &self,
        team_id: &str,
        type_or_name: &str,
    ) -> Result<WorkflowState> {
        let query = r#"
            query($teamId: ID!) {
                workflowStates(filter: { team: { id: { eq: $teamId } } }) {
                    nodes {
                        id
                        name
                        type
                    }
                }
            }
        "#;

        let variables = serde_json::json!({ "teamId": team_id });
        let response: WorkflowStatesResponse = self.query(query, Some(variables)).await?;

        let type_or_name_lower = type_or_name.to_lowercase();

        // Try to match by type first (stable identifiers)
        if let Some(state) = response
            .workflow_states
            .nodes
            .iter()
            .find(|s| s.state_type.to_lowercase() == type_or_name_lower)
        {
            return Ok(state.clone());
        }

        // Fall back to matching by name
        response
            .workflow_states
            .nodes
            .into_iter()
            .find(|s| s.name.as_ref().map(|n| n.to_lowercase()) == Some(type_or_name_lower.clone()))
            .ok_or_else(|| anyhow::anyhow!("No workflow state matching '{}' found", type_or_name))
    }

    /// Transition an issue to a workflow state
    pub async fn transition_issue(
        &self,
        team_id: &str,
        issue_number: u64,
        state_type_or_name: &str,
    ) -> Result<()> {
        let issue = self.get_issue_by_number(team_id, issue_number).await?;
        let state = self
            .get_state_by_type_or_name(team_id, state_type_or_name)
            .await?;

        let query = r#"
            mutation($issueId: String!, $stateId: String!) {
                issueUpdate(id: $issueId, input: { stateId: $stateId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "stateId": state.id
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to transition issue");
        }
        Ok(())
    }
}
