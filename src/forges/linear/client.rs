//! Linear GraphQL client implementation

use std::sync::RwLock;

use anyhow::Result;

use super::types::*;
use crate::forges::create_http_client;

/// Linear GraphQL client
pub struct LinearClient {
    pub(super) client: reqwest::Client,
    pub(super) token: RwLock<String>,
}

impl LinearClient {
    pub fn new(token: String) -> Self {
        Self {
            client: create_http_client(),
            token: RwLock::new(token),
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
}
