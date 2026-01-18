//! JIRA API client implementation

use std::sync::RwLock;

use anyhow::{Result, anyhow};
use base64::Engine;
use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use super::oauth::{JiraAuthMode, JiraCredentials, refresh_token};
use super::types::JiraProject;
use super::types::JiraUser;
use super::{AUTH, parse_jira_error};

/// JIRA API client
pub struct JiraClient {
    client: reqwest::Client,
    pub(super) creds: RwLock<JiraCredentials>,
}

impl JiraClient {
    pub fn new(creds: JiraCredentials) -> Self {
        Self {
            client: reqwest::Client::new(),
            creds: RwLock::new(creds),
        }
    }

    /// Get the base URL for JIRA REST API v3
    fn api_base(&self) -> String {
        let creds = self.creds.read().unwrap();
        match &creds.auth_mode {
            JiraAuthMode::OAuth { cloud_id } => {
                format!("https://api.atlassian.com/ex/jira/{}/rest/api/3", cloud_id)
            }
            JiraAuthMode::ApiToken { .. } => {
                format!("{}/rest/api/3", creds.site_url)
            }
        }
    }

    /// Get the auth header value
    fn auth_header(&self) -> (String, String) {
        let creds = self.creds.read().unwrap();
        match &creds.auth_mode {
            JiraAuthMode::OAuth { .. } => ("Bearer".to_string(), creds.access_token.clone()),
            JiraAuthMode::ApiToken { email } => {
                // Basic auth: base64(email:token)
                let basic = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", email, creds.access_token));
                ("Basic".to_string(), basic)
            }
        }
    }

    /// Get the site URL for building browse links
    pub fn site_url(&self) -> String {
        let creds = self.creds.read().unwrap();
        creds.site_url.clone()
    }

    /// Make an authenticated GET request
    pub async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        trace!(method = "GET", path, "JIRA API request");

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .send()
            .await?;

        let status = response.status();
        trace!(status = %status, "JIRA API response");

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "JIRA authentication failed. Please re-authenticate with: isq link jira"
            ));
        }

        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "Access denied. You may not have permission to access this JIRA project."
            ));
        }

        if !status.is_success() {
            let body = response.text().await?;
            debug!(status = %status, body = %body, "JIRA API request failed");
            return Err(anyhow!("JIRA API error ({}): {}", status, body));
        }

        let result = response.json().await?;
        Ok(result)
    }

    /// Make an authenticated POST request
    pub async fn post<T: for<'de> Deserialize<'de>, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        trace!(method = "POST", path, "JIRA API request");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        let status = response.status();
        trace!(status = %status, "JIRA API response");

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "JIRA authentication failed. Please re-authenticate with: isq link jira"
            ));
        }

        if !status.is_success() {
            let body = response.text().await?;
            debug!(status = %status, body = %body, "JIRA API request failed");
            return Err(parse_jira_error(status, &body));
        }

        let result = response.json().await?;
        Ok(result)
    }

    /// Make an authenticated POST request without expecting a response body
    pub async fn post_no_response<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        trace!(method = "POST", path, "JIRA API request (no response)");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        let status = response.status();
        trace!(status = %status, "JIRA API response");

        if !status.is_success() {
            let body = response.text().await?;
            debug!(status = %status, body = %body, "JIRA API request failed");
            return Err(parse_jira_error(status, &body));
        }

        Ok(())
    }

    /// Make an authenticated PUT request
    pub async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        trace!(method = "PUT", path, "JIRA API request");

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        let status = response.status();
        trace!(status = %status, "JIRA API response");

        if !status.is_success() {
            let body = response.text().await?;
            debug!(status = %status, body = %body, "JIRA API request failed");
            return Err(anyhow!("JIRA API error ({}): {}", status, body));
        }

        Ok(())
    }

    /// Refresh the access token if needed (only for OAuth mode)
    async fn refresh_if_needed(&self) -> Result<()> {
        let needs_refresh = {
            let creds = self.creds.read().unwrap();
            // API tokens don't expire, only refresh OAuth tokens
            if matches!(creds.auth_mode, JiraAuthMode::ApiToken { .. }) {
                return Ok(());
            }
            if let Some(expires_at) = creds.expires_at {
                let now = chrono::Utc::now().timestamp();
                let remaining = expires_at - now;
                // Refresh if less than 5 minutes remaining
                remaining < 300
            } else {
                false
            }
        };

        if needs_refresh {
            self.do_refresh_token().await?;
        }

        Ok(())
    }

    /// Refresh the access token using the stored refresh token
    async fn do_refresh_token(&self) -> Result<()> {
        let stored_refresh_token = {
            let creds = self.creds.read().unwrap();
            creds.refresh_token.clone().ok_or_else(|| {
                anyhow!("No refresh token available - please re-authenticate with: isq link jira")
            })?
        };

        let new_tokens = refresh_token(&stored_refresh_token).await?;

        let expires_at = new_tokens
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs as i64);

        // Update stored credentials
        {
            let mut creds = self.creds.write().unwrap();
            creds.access_token = new_tokens.access_token.clone();
            if let Some(rt) = &new_tokens.refresh_token {
                creds.refresh_token = Some(rt.clone());
            }
            creds.expires_at = expires_at;
        }

        // Store updated credentials in keyring (only for OAuth mode)
        let creds = self.creds.read().unwrap();
        if let JiraAuthMode::OAuth { cloud_id } = &creds.auth_mode {
            let cred_json = serde_json::json!({
                "access_token": creds.access_token,
                "refresh_token": creds.refresh_token,
                "cloud_id": cloud_id,
                "site_url": creds.site_url,
                "expires_at": creds.expires_at
            });
            AUTH.store_credential(&cred_json.to_string(), None, None)?;
        }

        Ok(())
    }

    /// List projects accessible to the user
    pub async fn list_projects(&self) -> Result<Vec<JiraProject>> {
        #[derive(Deserialize)]
        struct ProjectsResponse {
            values: Vec<JiraProject>,
        }

        let response: ProjectsResponse = self.get("/project/search?maxResults=100").await?;
        Ok(response.values)
    }

    /// Get current user info
    pub async fn get_current_user(&self) -> Result<JiraUser> {
        self.get("/myself").await
    }

    /// Check if user has write permissions using /mypermissions endpoint
    pub async fn check_write_permission(&self, project_key: &str) -> Result<bool> {
        let path = format!(
            "/mypermissions?projectKey={}&permissions=CREATE_ISSUES",
            project_key
        );

        #[derive(Deserialize)]
        struct PermissionsResponse {
            permissions: std::collections::HashMap<String, Permission>,
        }

        #[derive(Deserialize)]
        struct Permission {
            #[serde(rename = "havePermission")]
            have_permission: bool,
        }

        match self.get::<PermissionsResponse>(&path).await {
            Ok(resp) => {
                let can_create = resp
                    .permissions
                    .get("CREATE_ISSUES")
                    .map(|p| p.have_permission)
                    .unwrap_or(false);
                Ok(can_create)
            }
            Err(e) if e.to_string().contains("403") || e.to_string().contains("Access denied") => {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}
