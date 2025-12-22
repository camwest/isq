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

    async fn get_issue(&self, _repo: &Repo, _number: u64) -> Result<Issue> {
        anyhow::bail!("Linear get_issue not implemented yet")
    }

    async fn get_user(&self) -> Result<String> {
        self.get_viewer().await
    }

    async fn create_issue(&self, _repo: &Repo, _req: CreateIssueRequest) -> Result<Issue> {
        anyhow::bail!("Linear create_issue not implemented yet")
    }

    async fn create_comment(&self, _repo: &Repo, _issue_number: u64, _body: &str) -> Result<()> {
        anyhow::bail!("Linear create_comment not implemented yet")
    }

    async fn close_issue(&self, _repo: &Repo, _issue_number: u64) -> Result<()> {
        anyhow::bail!("Linear close_issue not implemented yet")
    }

    async fn reopen_issue(&self, _repo: &Repo, _issue_number: u64) -> Result<()> {
        anyhow::bail!("Linear reopen_issue not implemented yet")
    }

    async fn add_label(&self, _repo: &Repo, _issue_number: u64, _label: &str) -> Result<()> {
        anyhow::bail!("Linear add_label not implemented yet")
    }

    async fn remove_label(&self, _repo: &Repo, _issue_number: u64, _label: &str) -> Result<()> {
        anyhow::bail!("Linear remove_label not implemented yet")
    }

    async fn assign_issue(&self, _repo: &Repo, _issue_number: u64, _assignee: &str) -> Result<()> {
        anyhow::bail!("Linear assign_issue not implemented yet")
    }
}
