//! Linear authentication and token refresh

use anyhow::{Result, anyhow};
use serde::Deserialize;

use super::LinearClient;
use super::oauth::refresh_token;
use super::types::GraphQLRequest;
use super::{AUTH, GRAPHQL_URL};

impl LinearClient {
    /// Execute a GraphQL query (internal, no retry)
    pub(super) async fn query_internal<T: for<'de> Deserialize<'de>>(
        &self,
        query: &str,
        variables: Option<serde_json::Value>,
    ) -> Result<T> {
        use super::types::GraphQLResponse;

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
            anyhow::bail!(
                "Linear API error {} Unauthorized: {}",
                status.as_u16(),
                body
            );
        }

        let result: GraphQLResponse<T> = response.json().await?;

        if let Some(errors) = result.errors {
            let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
            anyhow::bail!("Linear GraphQL errors: {}", messages.join(", "));
        }

        result
            .data
            .ok_or_else(|| anyhow::anyhow!("No data in response"))
    }

    /// Refresh the access token using the stored refresh token
    pub(super) async fn do_refresh_token(&self) -> Result<()> {
        let cred = AUTH
            .get_credential()?
            .ok_or_else(|| anyhow!("No Linear credentials found"))?;

        let stored_refresh_token = cred.refresh_token.ok_or_else(|| {
            anyhow!("No refresh token available - please re-authenticate with: isq link linear")
        })?;

        let new_tokens = refresh_token(&stored_refresh_token).await?;

        // Update stored credentials in OS keyring
        let expires_at = new_tokens
            .expires_in
            .map(|secs| (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());
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
}
