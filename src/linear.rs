use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::forge::{CreateIssueRequest, Forge};
use crate::github::{Issue, Label, User};
use crate::repo::Repo;

const GRAPHQL_URL: &str = "https://api.linear.app/graphql";

/// Linear GraphQL client
#[derive(Clone)]
pub struct LinearClient {
    client: reqwest::Client,
    token: String,
}

// GraphQL response types

#[derive(Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Deserialize)]
struct ViewerResponse {
    viewer: LinearUser,
}

#[derive(Deserialize)]
struct TeamsResponse {
    teams: TeamConnection,
}

#[derive(Deserialize)]
struct TeamConnection {
    nodes: Vec<LinearTeam>,
}

#[derive(Deserialize, Clone)]
pub struct LinearTeam {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Deserialize)]
struct OrganizationResponse {
    organization: LinearOrganization,
}

#[derive(Deserialize, Clone)]
pub struct LinearOrganization {
    #[serde(rename = "urlKey")]
    pub url_key: String,
    pub name: String,
}

#[derive(Deserialize)]
struct LinearUser {
    id: String,
    name: String,
    email: String,
}

#[derive(Deserialize)]
struct IssuesResponse {
    issues: IssueConnection,
}

#[derive(Deserialize)]
struct IssueConnection {
    nodes: Vec<LinearIssue>,
}

#[derive(Deserialize)]
struct LinearIssue {
    id: String,
    identifier: String,
    number: u64,
    title: String,
    description: Option<String>,
    state: LinearState,
    creator: Option<LinearCreator>,
    labels: LabelConnection,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Deserialize)]
struct LinearState {
    name: String,
    #[serde(rename = "type")]
    state_type: String,
}

#[derive(Deserialize)]
struct LinearCreator {
    name: String,
}

#[derive(Deserialize)]
struct LabelConnection {
    nodes: Vec<LinearLabel>,
}

#[derive(Deserialize)]
struct LinearLabel {
    name: String,
    color: String,
}

#[derive(Serialize)]
struct GraphQLRequest {
    query: String,
    variables: Option<serde_json::Value>,
}

// Mutation response types

#[derive(Deserialize)]
struct IssueCreateResponse {
    #[serde(rename = "issueCreate")]
    issue_create: IssueCreatePayload,
}

#[derive(Deserialize)]
struct IssueCreatePayload {
    issue: CreatedIssue,
}

#[derive(Deserialize)]
struct CreatedIssue {
    id: String,
    identifier: String,
    number: u64,
    title: String,
}

#[derive(Deserialize)]
struct CommentCreateResponse {
    #[serde(rename = "commentCreate")]
    comment_create: CommentCreatePayload,
}

#[derive(Deserialize)]
struct CommentCreatePayload {
    success: bool,
}

#[derive(Deserialize)]
struct IssueUpdateResponse {
    #[serde(rename = "issueUpdate")]
    issue_update: IssueUpdatePayload,
}

#[derive(Deserialize)]
struct IssueUpdatePayload {
    success: bool,
}

// Response types for fetching issues with comments
#[derive(Deserialize)]
struct IssuesWithCommentsResponse {
    issues: IssueWithCommentsConnection,
}

#[derive(Deserialize)]
struct IssueWithCommentsConnection {
    nodes: Vec<IssueWithComments>,
}

#[derive(Deserialize)]
struct IssueWithComments {
    number: u64,
    comments: CommentConnection,
}

#[derive(Deserialize)]
struct CommentConnection {
    nodes: Vec<LinearComment>,
}

#[derive(Deserialize)]
struct LinearComment {
    id: String,
    body: String,
    user: Option<LinearCommentUser>,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[derive(Deserialize)]
struct LinearCommentUser {
    name: String,
}

#[derive(Deserialize)]
struct SingleIssueResponse {
    issue: Option<LinearIssueWithDetails>,
}

#[derive(Deserialize)]
struct LinearIssueWithDetails {
    id: String,
    identifier: String,
    number: u64,
    title: String,
    description: Option<String>,
    state: LinearState,
    creator: Option<LinearCreator>,
    labels: LabelConnectionWithIds,
    assignee: Option<LinearAssignee>,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Deserialize)]
struct LabelConnectionWithIds {
    nodes: Vec<LinearLabelWithId>,
}

#[derive(Deserialize)]
struct LinearLabelWithId {
    id: String,
    name: String,
    color: String,
}

#[derive(Deserialize)]
struct LinearAssignee {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct WorkflowStatesResponse {
    #[serde(rename = "workflowStates")]
    workflow_states: WorkflowStateConnection,
}

#[derive(Deserialize)]
struct WorkflowStateConnection {
    nodes: Vec<WorkflowState>,
}

#[derive(Deserialize)]
struct WorkflowState {
    id: String,
    name: String,
    #[serde(rename = "type")]
    state_type: String,
}

#[derive(Deserialize)]
struct UsersResponse {
    users: UserConnection,
}

#[derive(Deserialize)]
struct UserConnection {
    nodes: Vec<LinearUserWithId>,
}

#[derive(Deserialize)]
struct LinearUserWithId {
    id: String,
    name: String,
    email: String,
}

#[derive(Deserialize)]
struct TeamLabelsResponse {
    team: TeamWithLabels,
}

#[derive(Deserialize)]
struct TeamWithLabels {
    labels: TeamLabelConnection,
}

#[derive(Deserialize)]
struct TeamLabelConnection {
    nodes: Vec<LinearLabelWithId>,
}

impl LinearClient {
    pub fn new(token: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            token,
        }
    }

    /// Execute a GraphQL query
    async fn query<T: for<'de> Deserialize<'de>>(&self, query: &str, variables: Option<serde_json::Value>) -> Result<T> {
        let request = GraphQLRequest {
            query: query.to_string(),
            variables,
        };

        let response = self
            .client
            .post(GRAPHQL_URL)
            .header("Authorization", &self.token)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("Linear API error {}: {}", status, body);
        }

        let result: GraphQLResponse<T> = response.json().await?;

        if let Some(errors) = result.errors {
            let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            anyhow::bail!("Linear GraphQL errors: {}", messages.join(", "));
        }

        result.data.ok_or_else(|| anyhow::anyhow!("No data in response"))
    }

    /// Get the authenticated user
    pub async fn get_viewer(&self) -> Result<String> {
        let query = r#"
            query {
                viewer {
                    id
                    name
                    email
                }
            }
        "#;

        let response: ViewerResponse = self.query(query, None).await?;
        Ok(response.viewer.name)
    }

    /// List all teams
    pub async fn list_teams(&self) -> Result<Vec<LinearTeam>> {
        let query = r#"
            query {
                teams {
                    nodes {
                        id
                        name
                        key
                    }
                }
            }
        "#;

        let response: TeamsResponse = self.query(query, None).await?;
        Ok(response.teams.nodes)
    }

    /// Get organization info (for workspace URL key)
    pub async fn get_organization(&self) -> Result<LinearOrganization> {
        let query = r#"
            query {
                organization {
                    urlKey
                    name
                }
            }
        "#;

        let response: OrganizationResponse = self.query(query, None).await?;
        Ok(response.organization)
    }

    /// Get issue by number within a team
    async fn get_issue_by_number(&self, team_id: &str, number: u64) -> Result<LinearIssueWithDetails> {
        let query = r#"
            query($teamId: ID!, $number: Float!) {
                issues(filter: { team: { id: { eq: $teamId } }, number: { eq: $number } }, first: 1) {
                    nodes {
                        id
                        identifier
                        number
                        title
                        description
                        state { name type }
                        creator { name }
                        labels { nodes { id name color } }
                        assignee { id name }
                        createdAt
                        updatedAt
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "teamId": team_id,
            "number": number as f64
        });

        let response: IssuesResponse = self.query(query, Some(variables)).await?;

        // Re-fetch with full details since we need the assignee field
        if let Some(issue) = response.issues.nodes.into_iter().next() {
            let detail_query = r#"
                query($issueId: String!) {
                    issue(id: $issueId) {
                        id
                        identifier
                        number
                        title
                        description
                        state { name type }
                        creator { name }
                        labels { nodes { id name color } }
                        assignee { id name }
                        createdAt
                        updatedAt
                    }
                }
            "#;
            let detail_vars = serde_json::json!({ "issueId": issue.id });
            let detail_response: SingleIssueResponse = self.query(detail_query, Some(detail_vars)).await?;
            detail_response.issue.ok_or_else(|| anyhow::anyhow!("Issue #{} not found", number))
        } else {
            anyhow::bail!("Issue #{} not found in team", number)
        }
    }

    /// Get workflow state by type (completed, started, backlog, etc.)
    async fn get_state_by_type(&self, team_id: &str, state_type: &str) -> Result<WorkflowState> {
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

        response.workflow_states.nodes
            .into_iter()
            .find(|s| s.state_type == state_type)
            .ok_or_else(|| anyhow::anyhow!("No workflow state of type '{}' found", state_type))
    }

    /// Get user by name or email
    async fn get_user_by_name(&self, name: &str) -> Result<LinearUserWithId> {
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
        response.users.nodes
            .into_iter()
            .find(|u| u.name.to_lowercase() == name_lower || u.email.to_lowercase() == name_lower)
            .ok_or_else(|| anyhow::anyhow!("User '{}' not found", name))
    }

    /// Get labels by name for a team
    async fn get_label_ids(&self, team_id: &str, label_names: &[String]) -> Result<Vec<String>> {
        let query = r#"
            query($teamId: ID!) {
                team(id: $teamId) {
                    labels {
                        nodes {
                            id
                            name
                            color
                        }
                    }
                }
            }
        "#;

        let variables = serde_json::json!({ "teamId": team_id });
        let response: TeamLabelsResponse = self.query(query, Some(variables)).await?;

        let mut label_ids = Vec::new();
        for name in label_names {
            let name_lower = name.to_lowercase();
            if let Some(label) = response.team.labels.nodes.iter()
                .find(|l| l.name.to_lowercase() == name_lower)
            {
                label_ids.push(label.id.clone());
            }
            // Silently skip labels that don't exist
        }
        Ok(label_ids)
    }

    /// List issues for a team
    pub async fn list_team_issues(&self, team_id: &str) -> Result<Vec<Issue>> {
        let query = r#"
            query($teamId: ID!) {
                issues(filter: { team: { id: { eq: $teamId } }, state: { type: { nin: ["canceled", "completed"] } } }, first: 250) {
                    nodes {
                        id
                        identifier
                        number
                        title
                        description
                        state {
                            name
                            type
                        }
                        creator {
                            name
                        }
                        labels {
                            nodes {
                                name
                                color
                            }
                        }
                        createdAt
                        updatedAt
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "teamId": team_id
        });

        let response: IssuesResponse = self.query(query, Some(variables)).await?;

        // Convert Linear issues to our Issue format
        let issues = response.issues.nodes.into_iter().map(|i| Issue {
            number: i.number,
            title: format!("{} {}", i.identifier, i.title),
            body: i.description,
            state: if i.state.state_type == "completed" || i.state.state_type == "canceled" {
                "closed".to_string()
            } else {
                "open".to_string()
            },
            user: User {
                login: i.creator.map(|c| c.name).unwrap_or_else(|| "unknown".to_string()),
            },
            labels: i.labels.nodes.into_iter().map(|l| Label {
                name: l.name,
                color: l.color.trim_start_matches('#').to_string(),
            }).collect(),
            created_at: i.created_at,
            updated_at: i.updated_at,
        }).collect();

        Ok(issues)
    }
}

#[async_trait]
impl Forge for LinearClient {
    async fn list_issues(&self, repo: &Repo) -> Result<Vec<Issue>> {
        // For Linear, repo.owner is ignored and repo.name is the team ID
        self.list_team_issues(&repo.name).await
    }

    async fn get_issue(&self, repo: &Repo, number: u64) -> Result<Issue> {
        let issue = self.get_issue_by_number(&repo.name, number).await?;
        Ok(Issue {
            number: issue.number,
            title: format!("{} {}", issue.identifier, issue.title),
            body: issue.description,
            state: if issue.state.state_type == "completed" || issue.state.state_type == "canceled" {
                "closed".to_string()
            } else {
                "open".to_string()
            },
            user: User {
                login: issue.creator.map(|c| c.name).unwrap_or_else(|| "unknown".to_string()),
            },
            labels: issue.labels.nodes.into_iter().map(|l| Label {
                name: l.name,
                color: l.color.trim_start_matches('#').to_string(),
            }).collect(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
        })
    }

    async fn get_user(&self) -> Result<String> {
        self.get_viewer().await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        let team_id = &repo.name;

        // Get label IDs if any labels specified
        let label_ids = if !req.labels.is_empty() {
            Some(self.get_label_ids(team_id, &req.labels).await?)
        } else {
            None
        };

        let query = r#"
            mutation($teamId: String!, $title: String!, $description: String, $labelIds: [String!]) {
                issueCreate(input: { teamId: $teamId, title: $title, description: $description, labelIds: $labelIds }) {
                    issue {
                        id
                        identifier
                        number
                        title
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "teamId": team_id,
            "title": req.title,
            "description": req.body,
            "labelIds": label_ids
        });

        let response: IssueCreateResponse = self.query(query, Some(variables)).await?;
        let created = response.issue_create.issue;

        Ok(Issue {
            number: created.number,
            title: format!("{} {}", created.identifier, created.title),
            body: req.body,
            state: "open".to_string(),
            user: User { login: "me".to_string() },
            labels: req.labels.into_iter().map(|name| Label {
                name,
                color: "888888".to_string(),
            }).collect(),
            created_at: String::new(), // Not returned by mutation
            updated_at: String::new(),
        })
    }

    async fn create_comment(&self, repo: &Repo, issue_number: u64, body: &str) -> Result<()> {
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;

        let query = r#"
            mutation($issueId: String!, $body: String!) {
                commentCreate(input: { issueId: $issueId, body: $body }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "body": body
        });

        let response: CommentCreateResponse = self.query(query, Some(variables)).await?;
        if !response.comment_create.success {
            anyhow::bail!("Failed to create comment");
        }
        Ok(())
    }

    async fn close_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        let done_state = self.get_state_by_type(&repo.name, "completed").await?;

        let query = r#"
            mutation($issueId: String!, $stateId: String!) {
                issueUpdate(id: $issueId, input: { stateId: $stateId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "stateId": done_state.id
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to close issue");
        }
        Ok(())
    }

    async fn reopen_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        // Try "backlog" first, fall back to "unstarted" or "started"
        let backlog_state = match self.get_state_by_type(&repo.name, "backlog").await {
            Ok(state) => state,
            Err(_) => match self.get_state_by_type(&repo.name, "unstarted").await {
                Ok(state) => state,
                Err(_) => self.get_state_by_type(&repo.name, "started").await?,
            }
        };

        let query = r#"
            mutation($issueId: String!, $stateId: String!) {
                issueUpdate(id: $issueId, input: { stateId: $stateId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "stateId": backlog_state.id
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to reopen issue");
        }
        Ok(())
    }

    async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        let label_ids = self.get_label_ids(&repo.name, &[label.to_string()]).await?;

        if label_ids.is_empty() {
            anyhow::bail!("Label '{}' not found", label);
        }

        // Get current label IDs and add the new one
        let mut current_ids: Vec<String> = issue.labels.nodes.iter().map(|l| l.id.clone()).collect();
        if !current_ids.contains(&label_ids[0]) {
            current_ids.push(label_ids[0].clone());
        }

        let query = r#"
            mutation($issueId: String!, $labelIds: [String!]!) {
                issueUpdate(id: $issueId, input: { labelIds: $labelIds }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "labelIds": current_ids
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to add label");
        }
        Ok(())
    }

    async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;

        // Get current label IDs and remove the specified one
        let label_lower = label.to_lowercase();
        let new_ids: Vec<String> = issue.labels.nodes.iter()
            .filter(|l| l.name.to_lowercase() != label_lower)
            .map(|l| l.id.clone())
            .collect();

        let query = r#"
            mutation($issueId: String!, $labelIds: [String!]!) {
                issueUpdate(id: $issueId, input: { labelIds: $labelIds }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "labelIds": new_ids
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to remove label");
        }
        Ok(())
    }

    async fn assign_issue(&self, repo: &Repo, issue_number: u64, assignee: &str) -> Result<()> {
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        let user = self.get_user_by_name(assignee).await?;

        let query = r#"
            mutation($issueId: String!, $assigneeId: String!) {
                issueUpdate(id: $issueId, input: { assigneeId: $assigneeId }) {
                    success
                }
            }
        "#;

        let variables = serde_json::json!({
            "issueId": issue.id,
            "assigneeId": user.id
        });

        let response: IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to assign issue");
        }
        Ok(())
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<Vec<crate::db::Comment>> {
        // Fetch all issues with their comments in a single query
        let query = r#"
            query($teamId: ID!) {
                issues(filter: { team: { id: { eq: $teamId } } }, first: 250) {
                    nodes {
                        number
                        comments {
                            nodes {
                                id
                                body
                                user {
                                    name
                                }
                                createdAt
                            }
                        }
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "teamId": repo.name
        });

        let response: IssuesWithCommentsResponse = self.query(query, Some(variables)).await?;

        // Flatten all comments from all issues
        let mut comments = Vec::new();
        for issue in response.issues.nodes {
            for comment in issue.comments.nodes {
                comments.push(crate::db::Comment {
                    comment_id: comment.id,
                    issue_number: issue.number,
                    body: comment.body,
                    author: comment.user.map(|u| u.name).unwrap_or_else(|| "unknown".to_string()),
                    created_at: comment.created_at,
                });
            }
        }

        Ok(comments)
    }
}
