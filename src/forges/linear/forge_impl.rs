//! Forge trait implementation for Linear

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::client::LinearClient;
use super::types;
use super::{
    GRAPHQL_URL, LinearOnCleanupConfig, LinearOnStartConfig, parse_issue_number,
    parse_reset_timestamp,
};
use crate::forges::{
    CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, Goal, Issue, Label, RateLimitInfo,
};
use crate::repo::Repo;

#[async_trait]
impl Forge for LinearClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        self.list_team_issues_internal(&repo.name, None).await
    }

    async fn list_issues_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<Issue>> {
        self.list_team_issues_internal(&repo.name, Some(since))
            .await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        let team_id = &repo.name;
        let org = self.get_organization().await?;

        let label_ids = if !req.labels.is_empty() {
            self.get_label_ids(team_id, &req.labels).await?
        } else {
            Vec::new()
        };

        let (query, variables) = if let Some(project_id) = &req.goal_id {
            let q = r#"
                mutation($teamId: String!, $title: String!, $description: String, $labelIds: [String!], $projectId: String!) {
                    issueCreate(input: { teamId: $teamId, title: $title, description: $description, labelIds: $labelIds, projectId: $projectId }) {
                        issue {
                            id
                            identifier
                            number
                            title
                        }
                    }
                }
            "#;
            let v = serde_json::json!({
                "teamId": team_id,
                "title": req.title,
                "description": req.body,
                "labelIds": label_ids,
                "projectId": project_id
            });
            (q, v)
        } else {
            let q = r#"
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
            let v = serde_json::json!({
                "teamId": team_id,
                "title": req.title,
                "description": req.body,
                "labelIds": label_ids
            });
            (q, v)
        };

        let response: types::IssueCreateResponse = self.query(query, Some(variables)).await?;
        let created = response.issue_create.issue;
        let url = format!(
            "https://linear.app/{}/issue/{}",
            org.url_key, created.identifier
        );

        Ok(Issue {
            id: created.identifier,
            title: created.title,
            body: req.body,
            state: "open".to_string(),
            author: "me".to_string(),
            labels: req.labels.into_iter().map(Label::name_only).collect(),
            assignees: vec![], // Not returned by mutation
            priority: 4,       // Default: none (not returned by mutation)
            priority_label: None,
            created_at: String::new(), // Not returned by mutation
            updated_at: String::new(),
            url: Some(url),
            milestone: req.goal_id.clone(),
            parent_id: req.parent_id.clone(),
        })
    }

    async fn create_comment(&self, repo: &Repo, issue_id: &str, body: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
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

        let response: types::CommentCreateResponse = self.query(query, Some(variables)).await?;
        if !response.comment_create.success {
            anyhow::bail!("Failed to create comment");
        }
        Ok(())
    }

    async fn close_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
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

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to close issue");
        }
        Ok(())
    }

    async fn reopen_issue(&self, repo: &Repo, issue_id: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        // Try "backlog" first, fall back to "unstarted" or "started"
        let backlog_state = match self.get_state_by_type(&repo.name, "backlog").await {
            Ok(state) => state,
            Err(_) => match self.get_state_by_type(&repo.name, "unstarted").await {
                Ok(state) => state,
                Err(_) => self.get_state_by_type(&repo.name, "started").await?,
            },
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

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to reopen issue");
        }
        Ok(())
    }

    async fn add_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        let label_ids = self.get_label_ids(&repo.name, &[label.to_string()]).await?;

        if label_ids.is_empty() {
            anyhow::bail!("Label '{}' not found", label);
        }

        let mut current_ids: Vec<String> =
            issue.labels.nodes.iter().map(|l| l.id.clone()).collect();
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

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to add label");
        }
        Ok(())
    }

    async fn remove_label(&self, repo: &Repo, issue_id: &str, label: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;

        let label_lower = label.to_lowercase();
        let new_ids: Vec<String> = issue
            .labels
            .nodes
            .iter()
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

        let response: types::IssueUpdateResponse = self.query(query, Some(variables)).await?;
        if !response.issue_update.success {
            anyhow::bail!("Failed to remove label");
        }
        Ok(())
    }

    async fn assign_issue(&self, repo: &Repo, issue_id: &str, assignee: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        let user = self.get_user_by_name(assignee).await?;
        self.assign_issue_by_id(&repo.name, issue_number, &user.id)
            .await
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<crate::db::Comment>> {
        self.list_comments_internal(&repo.name, None).await
    }

    async fn list_comments_since(
        &self,
        repo: &Repo,
        since: DateTime<Utc>,
    ) -> Result<FetchResult<crate::db::Comment>> {
        self.list_comments_internal(&repo.name, Some(since)).await
    }

    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        let projects = self.list_projects(&repo.name).await?;
        Ok(projects.into_iter().map(Goal::from).collect())
    }

    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal> {
        let project = self.create_project(&repo.name, &req).await?;
        Ok(Goal::from(project))
    }

    async fn close_goal(&self, _repo: &Repo, goal_id: &str) -> Result<()> {
        self.complete_project(goal_id).await
    }

    async fn assign_to_goal(&self, repo: &Repo, issue_id: &str, goal_id: &str) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;
        // Get the internal issue ID from the issue number
        let issue = self.get_issue_by_number(&repo.name, issue_number).await?;
        self.set_issue_project(&issue.id, goal_id).await
    }

    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        // Linear returns rate limit info in response headers
        // Make a minimal query to get the headers
        let request = types::GraphQLRequest {
            query: "query { viewer { id } }".to_string(),
            variables: None,
        };

        let token = self.token.read().unwrap().clone();
        let client = super::super::create_http_client();
        let response = client
            .post(GRAPHQL_URL)
            .header("Authorization", &token)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        // Extract rate limit headers
        // Linear uses: X-RateLimit-Requests-Limit, X-RateLimit-Requests-Remaining, X-RateLimit-Requests-Reset
        let limit = response
            .headers()
            .get("x-ratelimit-requests-limit")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1500); // Default to Linear's documented limit

        let remaining = response
            .headers()
            .get("x-ratelimit-requests-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok());

        let reset_at = response
            .headers()
            .get("x-ratelimit-requests-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_reset_timestamp);

        match (remaining, reset_at) {
            (Some(remaining), Some(reset_at)) => Ok(Some(RateLimitInfo {
                limit,
                remaining,
                reset_at,
            })),
            _ => Ok(None), // Headers not present, Linear may not always send them
        }
    }

    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        user_id: Option<&str>,
    ) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;

        // Parse Linear-specific config from opaque toml::Value
        let cfg: LinearOnStartConfig = config.clone().try_into().unwrap_or_default();

        // Transition to configured workflow state
        if let Some(ref transition) = cfg.transition {
            self.transition_issue(&repo.name, issue_number, transition)
                .await?;
        }

        // Assign to self if configured (user_id is the Linear user UUID)
        if cfg.assign_self
            && let Some(id) = user_id
        {
            self.assign_issue_by_id(&repo.name, issue_number, id)
                .await?;
        }

        Ok(())
    }

    async fn list_labels(&self, repo: &Repo) -> Result<Vec<Label>> {
        // For Linear, repo.name is the team ID
        let query = r#"
            query($teamId: ID!) {
                team(id: $teamId) {
                    labels {
                        nodes {
                            name
                            color
                        }
                    }
                }
            }
        "#;

        let variables = serde_json::json!({ "teamId": repo.name });

        #[derive(Deserialize)]
        struct TeamLabelsResponse {
            team: TeamLabels,
        }
        #[derive(Deserialize)]
        struct TeamLabels {
            labels: LabelNodes,
        }
        #[derive(Deserialize)]
        struct LabelNodes {
            nodes: Vec<types::LinearLabel>,
        }

        let response: TeamLabelsResponse = self.query(query, Some(variables)).await?;
        Ok(response
            .team
            .labels
            .nodes
            .into_iter()
            .map(|l| Label::new(l.name, Some(l.color)))
            .collect())
    }

    async fn create_label(
        &self,
        repo: &Repo,
        name: &str,
        color: Option<&str>,
        _description: Option<&str>,
    ) -> Result<Label> {
        // For Linear, repo.name is the team ID
        let query = r#"
            mutation($teamId: String!, $name: String!, $color: String) {
                issueLabelCreate(input: { teamId: $teamId, name: $name, color: $color }) {
                    issueLabel {
                        name
                        color
                    }
                }
            }
        "#;

        let color = color.map(|c| c.trim_start_matches('#').to_string());

        let variables = serde_json::json!({
            "teamId": repo.name,
            "name": name,
            "color": color
        });

        #[derive(Deserialize)]
        struct CreateLabelResponse {
            #[serde(rename = "issueLabelCreate")]
            issue_label_create: IssueLabelCreate,
        }
        #[derive(Deserialize)]
        struct IssueLabelCreate {
            #[serde(rename = "issueLabel")]
            issue_label: types::LinearLabel,
        }

        let response: CreateLabelResponse = self.query(query, Some(variables)).await?;
        let label = response.issue_label_create.issue_label;
        Ok(Label::new(label.name, Some(label.color)))
    }

    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()> {
        let _: LinearOnStartConfig = config.clone().try_into().context(
            "Invalid [on_start] config for Linear.\nValid fields: transition, assign_self",
        )?;
        Ok(())
    }

    async fn handle_on_cleanup(
        &self,
        repo: &Repo,
        issue_id: &str,
        config: &toml::Value,
        _user_id: Option<&str>,
    ) -> Result<()> {
        let issue_number = parse_issue_number(issue_id)?;

        // Parse Linear-specific config from opaque toml::Value
        let cfg: LinearOnCleanupConfig = config.clone().try_into().unwrap_or_default();

        // Transition to configured workflow state (optional)
        if let Some(ref transition) = cfg.transition
            && let Err(e) = self
                .transition_issue(&repo.name, issue_number, transition)
                .await
        {
            eprintln!("Warning: could not transition issue: {}", e);
        }

        Ok(())
    }

    fn validate_on_cleanup_config(&self, config: &toml::Value) -> Result<()> {
        let _: LinearOnCleanupConfig = config
            .clone()
            .try_into()
            .context("Invalid [on_cleanup] config for Linear.\nValid fields: transition")?;
        Ok(())
    }
}
