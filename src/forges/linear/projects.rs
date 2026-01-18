//! Linear project (goals) operations

use anyhow::Result;

use super::LinearClient;
use super::types::{
    IssueUpdateResponse, LinearProject, ProjectCreateResponse, ProjectUpdateResponse,
    ProjectsResponse,
};
use crate::forges::CreateGoalRequest;

impl LinearClient {
    /// List projects for a team
    pub async fn list_projects(&self, team_id: &str) -> Result<Vec<LinearProject>> {
        let query = r#"
            query($teamId: ID!) {
                projects(filter: { accessibleTeams: { id: { eq: $teamId } } }, first: 100) {
                    nodes {
                        id
                        name
                        description
                        state
                        targetDate
                        createdAt
                        updatedAt
                        url
                        progress
                    }
                }
            }
        "#;

        let variables = serde_json::json!({ "teamId": team_id });
        let response: ProjectsResponse = self.query(query, Some(variables)).await?;
        Ok(response.projects.nodes)
    }

    /// Create a new project
    pub async fn create_project(
        &self,
        team_id: &str,
        req: &CreateGoalRequest,
    ) -> Result<LinearProject> {
        let query = r#"
            mutation($input: ProjectCreateInput!) {
                projectCreate(input: $input) {
                    success
                    project {
                        id
                        name
                        description
                        state
                        targetDate
                        createdAt
                        updatedAt
                        url
                        progress
                    }
                }
            }
        "#;

        let mut input = serde_json::json!({
            "name": req.name,
            "teamIds": [team_id]
        });

        if let Some(desc) = &req.description {
            input["description"] = serde_json::json!(desc);
        }

        if let Some(date) = &req.target_date {
            input["targetDate"] = serde_json::json!(date);
        }

        let variables = serde_json::json!({ "input": input });
        let response: ProjectCreateResponse = self.query(query, Some(variables)).await?;

        if !response.project_create.success {
            anyhow::bail!("Failed to create project");
        }

        response
            .project_create
            .project
            .ok_or_else(|| anyhow::anyhow!("Project created but not returned"))
    }

    /// Update project state to completed
    pub async fn complete_project(&self, project_id: &str) -> Result<()> {
        let query = r#"
            mutation($id: String!, $input: ProjectUpdateInput!) {
                projectUpdate(id: $id, input: $input) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "id": project_id,
            "input": { "state": "completed" }
        });

        let response: ProjectUpdateResponse = self.query(query, Some(variables)).await?;

        if !response.project_update.success {
            anyhow::bail!("Failed to complete project");
        }

        Ok(())
    }

    /// Assign issue to project
    pub async fn set_issue_project(&self, issue_id: &str, project_id: &str) -> Result<()> {
        let query = r#"
            mutation($issueId: String!, $input: IssueUpdateInput!) {
                issueUpdate(id: $issueId, input: $input) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue_id,
            "input": { "projectId": project_id }
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;

        if !response.issue_update.success {
            anyhow::bail!("Failed to assign issue to project");
        }

        Ok(())
    }
}
