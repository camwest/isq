//! Linear GraphQL client implementation

use std::sync::RwLock;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::oauth::refresh_token;
use super::types::*;
use super::{map_linear_priority, AUTH, GRAPHQL_URL};
use crate::forges::{create_http_client, CreateGoalRequest, FetchResult, Issue, Label};
use crate::db;

/// Linear GraphQL client
pub struct LinearClient {
    client: reqwest::Client,
    pub(super) token: RwLock<String>,
}

impl LinearClient {
    pub fn new(token: String) -> Self {
        Self {
            client: create_http_client(),
            token: RwLock::new(token),
        }
    }

    /// Execute a GraphQL query (internal, no retry)
    async fn query_internal<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: Option<serde_json::Value>,
    ) -> Result<T> {
        let token = self.token.read().unwrap().clone();
        let request = GraphQLRequest {
            query: query.to_string(),
            variables,
        };

        let response = self
            .client
            .post(GRAPHQL_URL)
            .header("Authorization", &token)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("Linear API error {} Unauthorized: {}", status.as_u16(), body);
        }

        let result: GraphQLResponse<T> = response.json().await?;

        if let Some(errors) = result.errors {
            let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            anyhow::bail!("Linear GraphQL errors: {}", messages.join(", "));
        }

        result.data.ok_or_else(|| anyhow::anyhow!("No data in response"))
    }

    /// Refresh the access token using the stored refresh token
    async fn do_refresh_token(&self) -> Result<()> {
        let cred = AUTH.get_credential()?
            .ok_or_else(|| anyhow!("No Linear credentials found"))?;

        let stored_refresh_token = cred.refresh_token
            .ok_or_else(|| anyhow!("No refresh token available - please re-authenticate with: isq link linear"))?;

        let new_tokens = refresh_token(&stored_refresh_token).await?;

        // Update stored credentials in OS keyring
        let expires_at = new_tokens.expires_in.map(|secs| {
            (chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
                .to_rfc3339()
        });
        AUTH.store_credential(
            &new_tokens.access_token,
            new_tokens.refresh_token.as_deref(),
            expires_at.as_deref(),
        )?;

        // Update in-memory token
        *self.token.write().unwrap() = new_tokens.access_token;

        Ok(())
    }

    /// Execute a GraphQL query with automatic token refresh on 401
    pub async fn query<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: Option<serde_json::Value>,
    ) -> Result<T> {
        match self.query_internal(query, variables.clone()).await {
            Ok(result) => Ok(result),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("401") || err_str.contains("Unauthorized") {
                    // Try to refresh and retry once
                    self.do_refresh_token().await?;
                    self.query_internal(query, variables).await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Get the authenticated user's ID and display name
    pub async fn get_viewer(&self) -> Result<(String, String)> {
        let query = r#"
            query {
                viewer {
                    id
                    displayName
                }
            }
        "#;

        let response: ViewerResponse = self.query(query, None).await?;
        Ok((response.viewer.id, response.viewer.display_name))
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

    /// Get issue by number within a team (returns id and label IDs for mutations)
    pub async fn get_issue_by_number(&self, team_id: &str, number: u64) -> Result<LinearIssueWithDetails> {
        let query = r#"
            query($teamId: ID!, $number: Float!) {
                issues(filter: { team: { id: { eq: $teamId } }, number: { eq: $number } }, first: 1) {
                    nodes {
                        id
                        labels { nodes { id name } }
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "teamId": team_id,
            "number": number as f64
        });

        let response: SingleIssueListResponse = self.query(query, Some(variables)).await?;

        response.issues.nodes.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Issue #{} not found in team", number))
    }

    /// Get workflow state by type (completed, started, backlog, etc.)
    pub async fn get_state_by_type(&self, team_id: &str, state_type: &str) -> Result<WorkflowState> {
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

    /// Get workflow state by type OR name
    /// Tries matching by type first (stable), then by name (customizable)
    pub async fn get_state_by_type_or_name(&self, team_id: &str, type_or_name: &str) -> Result<WorkflowState> {
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
        if let Some(state) = response.workflow_states.nodes.iter()
            .find(|s| s.state_type.to_lowercase() == type_or_name_lower)
        {
            return Ok(state.clone());
        }

        // Fall back to matching by name
        response.workflow_states.nodes
            .into_iter()
            .find(|s| s.name.as_ref().map(|n| n.to_lowercase()) == Some(type_or_name_lower.clone()))
            .ok_or_else(|| anyhow::anyhow!("No workflow state matching '{}' found", type_or_name))
    }

    /// Transition an issue to a workflow state
    pub async fn transition_issue(&self, team_id: &str, issue_number: u64, state_type_or_name: &str) -> Result<()> {
        let issue = self.get_issue_by_number(team_id, issue_number).await?;
        let state = self.get_state_by_type_or_name(team_id, state_type_or_name).await?;

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
        response.users.nodes
            .into_iter()
            .find(|u| u.name.to_lowercase() == name_lower || u.email.to_lowercase() == name_lower)
            .ok_or_else(|| anyhow::anyhow!("User '{}' not found", name))
    }

    /// Assign issue by user ID directly (no name lookup)
    pub async fn assign_issue_by_id(&self, team_id: &str, issue_number: u64, user_id: &str) -> Result<()> {
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

    /// Get labels by name for a team
    pub async fn get_label_ids(&self, team_id: &str, label_names: &[String]) -> Result<Vec<String>> {
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

    /// List issues for a team (with pagination)
    /// Returns FetchResult with completeness tracking
    pub async fn list_team_issues_internal(&self, team_id: &str, since: Option<DateTime<Utc>>) -> Result<FetchResult<Issue>> {
        // Fetch org URL key for constructing issue URLs
        let org = self.get_organization().await?;
        let url_key = org.url_key;

        let mut all_issues = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            match self.fetch_issues_page(team_id, &url_key, cursor.as_deref(), since.as_ref()).await {
                Ok((issues, page_info)) => {
                    all_issues.extend(issues);
                    page += 1;
                    // Print progress every 10 pages
                    if page % 10 == 0 {
                        eprintln!("  {} issues...", all_issues.len());
                    }
                    if !page_info.has_next_page {
                        break;
                    }
                    cursor = page_info.end_cursor;
                }
                Err(e) => {
                    eprintln!("Warning: Linear issues page fetch failed: {}", e);
                    return Ok(FetchResult::incomplete(all_issues));
                }
            }
        }

        Ok(FetchResult::complete(all_issues))
    }

    /// Fetch a single page of issues
    /// When since is provided, uses updatedAt filter and orderBy for incremental sync
    async fn fetch_issues_page(&self, team_id: &str, url_key: &str, after: Option<&str>, since: Option<&DateTime<Utc>>) -> Result<(Vec<Issue>, PageInfo)> {
        // Use different query for incremental vs full sync
        let query = if since.is_some() {
            r#"
            query($teamId: ID!, $after: String, $since: DateTimeOrDuration!) {
                issues(
                    filter: { team: { id: { eq: $teamId } }, updatedAt: { gte: $since } },
                    orderBy: updatedAt,
                    first: 250,
                    after: $after
                ) {
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
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
                        assignee {
                            displayName
                        }
                        priority
                        labels {
                            nodes {
                                name
                                color
                            }
                        }
                        project {
                            name
                        }
                        createdAt
                        updatedAt
                    }
                }
            }
        "#
        } else {
            r#"
            query($teamId: ID!, $after: String) {
                issues(filter: { team: { id: { eq: $teamId } } }, first: 250, after: $after) {
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
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
                        assignee {
                            displayName
                        }
                        priority
                        labels {
                            nodes {
                                name
                                color
                            }
                        }
                        project {
                            name
                        }
                        createdAt
                        updatedAt
                    }
                }
            }
        "#
        };

        let variables = match since {
            Some(ts) => serde_json::json!({
                "teamId": team_id,
                "after": after,
                "since": ts.to_rfc3339()
            }),
            None => serde_json::json!({
                "teamId": team_id,
                "after": after
            }),
        };

        let response: IssuesResponse = self.query(query, Some(variables)).await?;

        let page_info = response.issues.page_info
            .unwrap_or(PageInfo { has_next_page: false, end_cursor: None });

        // Convert Linear issues to our Issue format
        let issues = response.issues.nodes.into_iter().map(|i| {
            let url = format!("https://linear.app/{}/issue/{}", url_key, i.identifier);
            let priority = map_linear_priority(i.priority);
            Issue {
                number: i.number,
                key: None,
                title: format!("{} {}", i.identifier, i.title),
                body: i.description,
                state: if i.state.state_type == "completed" || i.state.state_type == "canceled" {
                    "closed".to_string()
                } else {
                    "open".to_string()
                },
                author: i.creator.map(|c| c.name).unwrap_or_else(|| "unknown".to_string()),
                labels: i.labels.nodes.into_iter().map(|l| Label::new(l.name, Some(l.color))).collect(),
                assignees: i.assignee.map(|a| vec![a.display_name]).unwrap_or_default(),
                priority,
                priority_label: None, // Linear uses native priority, not labels
                created_at: i.created_at,
                updated_at: i.updated_at,
                url: Some(url),
                milestone: i.project.map(|p| p.name),
            }
        }).collect();

        Ok((issues, page_info))
    }

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
    pub async fn create_project(&self, team_id: &str, req: &CreateGoalRequest) -> Result<LinearProject> {
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

        response.project_create.project
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

    /// List all comments for a team with optional since filter
    /// Uses direct comments query with pagination
    pub async fn list_comments_internal(&self, team_id: &str, since: Option<DateTime<Utc>>) -> Result<FetchResult<db::Comment>> {
        let mut all_comments = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            let query = if since.is_some() {
                r#"
                query($teamId: ID!, $after: String, $since: DateTimeOrDuration!) {
                    comments(
                        filter: { issue: { team: { id: { eq: $teamId } } }, updatedAt: { gte: $since } },
                        orderBy: updatedAt,
                        first: 250,
                        after: $after
                    ) {
                        pageInfo {
                            hasNextPage
                            endCursor
                        }
                        nodes {
                            id
                            body
                            user {
                                name
                            }
                            issue {
                                number
                            }
                            createdAt
                            updatedAt
                        }
                    }
                }
            "#
            } else {
                r#"
                query($teamId: ID!, $after: String) {
                    comments(
                        filter: { issue: { team: { id: { eq: $teamId } } } },
                        first: 250,
                        after: $after
                    ) {
                        pageInfo {
                            hasNextPage
                            endCursor
                        }
                        nodes {
                            id
                            body
                            user {
                                name
                            }
                            issue {
                                number
                            }
                            createdAt
                            updatedAt
                        }
                    }
                }
            "#
            };

            let variables = match since {
                Some(ts) => serde_json::json!({
                    "teamId": team_id,
                    "after": cursor,
                    "since": ts.to_rfc3339()
                }),
                None => serde_json::json!({
                    "teamId": team_id,
                    "after": cursor
                }),
            };

            match self.query::<CommentsResponse>(query, Some(variables)).await {
                Ok(response) => {
                    for comment in response.comments.nodes {
                        all_comments.push(db::Comment {
                            comment_id: comment.id,
                            issue_number: comment.issue.number,
                            body: comment.body,
                            author: comment.user.map(|u| u.name).unwrap_or_else(|| "unknown".to_string()),
                            created_at: comment.created_at,
                            updated_at: Some(comment.updated_at),
                        });
                    }

                    page += 1;
                    // Print progress every 10 pages
                    if page % 10 == 0 {
                        eprintln!("  {} comments...", all_comments.len());
                    }

                    let page_info = response.comments.page_info
                        .unwrap_or(PageInfo { has_next_page: false, end_cursor: None });

                    if !page_info.has_next_page {
                        break;
                    }
                    cursor = page_info.end_cursor;
                }
                Err(e) => {
                    eprintln!("Warning: Linear comments page fetch failed: {}", e);
                    return Ok(FetchResult::incomplete(all_comments));
                }
            }
        }

        Ok(FetchResult::complete(all_comments))
    }
}
