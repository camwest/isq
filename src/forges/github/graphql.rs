//! GitHub GraphQL API for efficient issue fetching with parent info

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::forges::{FetchResult, Issue, Label};
use crate::repo::Repo;

use super::GitHubClient;

const GRAPHQL_URL: &str = "https://api.github.com/graphql";

// ============================================================================
// GraphQL Request/Response Types
// ============================================================================

#[derive(Serialize)]
struct GraphQLRequest {
    query: String,
    variables: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize)]
struct GraphQLError {
    message: String,
}

// ============================================================================
// Issue Response Types
// ============================================================================

#[derive(Deserialize, Clone)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct GraphQLIssue {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    author: Option<Author>,
    labels: Option<LabelConnection>,
    assignees: AssigneeConnection,
    milestone: Option<MilestoneRef>,
    parent: Option<ParentRef>,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    url: String,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

#[derive(Deserialize)]
struct LabelConnection {
    nodes: Vec<GraphQLLabel>,
}

#[derive(Deserialize)]
struct GraphQLLabel {
    name: String,
    color: String,
}

#[derive(Deserialize)]
struct AssigneeConnection {
    nodes: Vec<Assignee>,
}

#[derive(Deserialize)]
struct Assignee {
    login: String,
}

#[derive(Deserialize)]
struct MilestoneRef {
    title: String,
}

#[derive(Deserialize)]
struct ParentRef {
    number: u64,
}

impl GraphQLIssue {
    fn into_issue(self) -> Issue {
        Issue {
            id: self.number.to_string(),
            title: self.title,
            body: self.body,
            state: self.state.to_lowercase(),
            author: self
                .author
                .map(|a| a.login)
                .unwrap_or_else(|| "unknown".to_string()),
            labels: self
                .labels
                .map(|l| {
                    l.nodes
                        .into_iter()
                        .map(|l| Label::new(l.name, Some(l.color)))
                        .collect()
                })
                .unwrap_or_default(),
            assignees: self.assignees.nodes.into_iter().map(|a| a.login).collect(),
            priority: 4, // Default: none (will be overridden if priority config exists)
            priority_label: None,
            created_at: self.created_at,
            updated_at: self.updated_at,
            url: Some(self.url),
            milestone: self.milestone.map(|m| m.title),
            parent_id: self.parent.map(|p| p.number.to_string()),
        }
    }
}

// ============================================================================
// GraphQL Client Implementation
// ============================================================================

impl GitHubClient {
    /// Execute a GraphQL query with sub-issues feature header
    async fn graphql_query<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: Option<serde_json::Value>,
    ) -> Result<T> {
        let request = GraphQLRequest {
            query: query.to_string(),
            variables,
        };

        let response = self
            .http_client()
            .post(GRAPHQL_URL)
            .header("Authorization", format!("Bearer {}", self.token()))
            .header("User-Agent", "isq")
            .header("Content-Type", "application/json")
            // Required for sub-issues API access
            .header("GraphQL-Features", "sub_issues")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("GitHub GraphQL error {}: {}", status, body);
        }

        let result: GraphQLResponse<T> = response.json().await?;

        if let Some(errors) = result.errors {
            let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            anyhow::bail!("GitHub GraphQL errors: {}", messages.join(", "));
        }

        result
            .data
            .ok_or_else(|| anyhow::anyhow!("GitHub GraphQL returned no data"))
    }

    /// Fetch issues using GraphQL (includes parent info inline)
    pub async fn list_issues_graphql(
        &self,
        repo: &Repo,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<Issue>> {
        let mut all_issues = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            match self
                .fetch_issues_page_graphql(repo, cursor.as_deref(), since.as_ref())
                .await
            {
                Ok((issues, page_info)) => {
                    all_issues.extend(issues);
                    page += 1;
                    if page % 10 == 0 {
                        eprintln!("  {} issues...", all_issues.len());
                    }
                    if !page_info.has_next_page {
                        break;
                    }
                    cursor = page_info.end_cursor;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    // Rate limit errors should be propagated
                    if err_str.contains("rate limit") || err_str.contains("429") {
                        return Err(e);
                    }
                    eprintln!("Warning: GraphQL page fetch failed: {}", err_str);
                    return Ok(FetchResult::incomplete(all_issues));
                }
            }
        }

        Ok(FetchResult::complete(all_issues))
    }

    /// Fetch a single page of issues via GraphQL
    async fn fetch_issues_page_graphql(
        &self,
        repo: &Repo,
        after: Option<&str>,
        since: Option<&DateTime<Utc>>,
    ) -> Result<(Vec<Issue>, PageInfo)> {
        // Build filter query string for since parameter
        let filter_query = match since {
            Some(ts) => format!(
                "repo:{}/{} is:issue updated:>{}",
                repo.owner,
                repo.name,
                ts.format("%Y-%m-%dT%H:%M:%SZ")
            ),
            None => format!("repo:{}/{} is:issue", repo.owner, repo.name),
        };

        // Note: We use search instead of repository.issues because search supports
        // filtering by updated date, which is needed for incremental sync.
        // The parent field requires the GraphQL-Features: sub_issues header.
        let query = r#"
            query($query: String!, $after: String) {
                search(query: $query, type: ISSUE, first: 100, after: $after) {
                    pageInfo {
                        hasNextPage
                        endCursor
                    }
                    nodes {
                        ... on Issue {
                            number
                            title
                            body
                            state
                            author { login }
                            labels(first: 100) { nodes { name color } }
                            assignees(first: 10) { nodes { login } }
                            milestone { title }
                            parent { number }
                            createdAt
                            updatedAt
                            url
                        }
                    }
                }
            }
        "#;

        let variables = serde_json::json!({
            "query": filter_query,
            "after": after
        });

        #[derive(Deserialize)]
        struct SearchResponse {
            search: SearchConnection,
        }

        #[derive(Deserialize)]
        struct SearchConnection {
            nodes: Vec<Option<GraphQLIssue>>,
            #[serde(rename = "pageInfo")]
            page_info: PageInfo,
        }

        let response: SearchResponse = self.graphql_query(query, Some(variables)).await?;

        let issues = response
            .search
            .nodes
            .into_iter()
            .flatten() // Filter out nulls (happens when node is not an Issue)
            .map(|i| i.into_issue())
            .collect();

        Ok((issues, response.search.page_info))
    }
}
