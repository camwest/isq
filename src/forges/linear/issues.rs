//! Linear issue operations

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::LinearClient;
use super::map_linear_priority;
use super::types::{
    IssuesResponse, LinearIssueWithDetails, PageInfo, SingleIssueListResponse, TeamLabelsResponse,
};
use crate::forges::{FetchResult, Issue, Label};

impl LinearClient {
    /// Get issue by number within a team (returns id and label IDs for mutations)
    pub async fn get_issue_by_number(
        &self,
        team_id: &str,
        number: u64,
    ) -> Result<LinearIssueWithDetails> {
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

        response
            .issues
            .nodes
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Issue #{} not found in team", number))
    }

    /// Get labels by name for a team
    pub async fn get_label_ids(
        &self,
        team_id: &str,
        label_names: &[String],
    ) -> Result<Vec<String>> {
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
            if let Some(label) = response
                .team
                .labels
                .nodes
                .iter()
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
    pub async fn list_team_issues_internal(
        &self,
        team_id: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<FetchResult<Issue>> {
        // Fetch org URL key for constructing issue URLs
        let org = self.get_organization().await?;
        let url_key = org.url_key;

        let mut all_issues = Vec::new();
        let mut cursor: Option<String> = None;
        let mut page = 0;

        loop {
            match self
                .fetch_issues_page(team_id, &url_key, cursor.as_deref(), since.as_ref())
                .await
            {
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
                    let err_str = e.to_string();
                    // Rate limit errors (429) should be propagated, not swallowed
                    if err_str.contains("429") {
                        return Err(e);
                    }
                    // Other errors: return partial data
                    eprintln!("Warning: Linear issues page fetch failed: {}", err_str);
                    return Ok(FetchResult::incomplete(all_issues));
                }
            }
        }

        Ok(FetchResult::complete(all_issues))
    }

    /// Fetch a single page of issues
    /// When since is provided, uses updatedAt filter and orderBy for incremental sync
    pub(super) async fn fetch_issues_page(
        &self,
        team_id: &str,
        url_key: &str,
        after: Option<&str>,
        since: Option<&DateTime<Utc>>,
    ) -> Result<(Vec<Issue>, PageInfo)> {
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

        let page_info = response.issues.page_info.unwrap_or(PageInfo {
            has_next_page: false,
            end_cursor: None,
        });

        // Convert Linear issues to our Issue format
        let issues = response
            .issues
            .nodes
            .into_iter()
            .map(|i| {
                let url = format!("https://linear.app/{}/issue/{}", url_key, i.identifier);
                let priority = map_linear_priority(i.priority);
                Issue {
                    id: i.identifier,
                    title: i.title,
                    body: i.description,
                    state: if i.state.state_type == "completed" || i.state.state_type == "canceled"
                    {
                        "closed".to_string()
                    } else {
                        "open".to_string()
                    },
                    author: i
                        .creator
                        .map(|c| c.name)
                        .unwrap_or_else(|| "unknown".to_string()),
                    labels: i
                        .labels
                        .nodes
                        .into_iter()
                        .map(|l| Label::new(l.name, Some(l.color)))
                        .collect(),
                    assignees: i.assignee.map(|a| vec![a.display_name]).unwrap_or_default(),
                    priority,
                    priority_label: None, // Linear uses native priority, not labels
                    created_at: i.created_at,
                    updated_at: i.updated_at,
                    url: Some(url),
                    milestone: i.project.map(|p| p.name),
                }
            })
            .collect();

        Ok((issues, page_info))
    }
}
