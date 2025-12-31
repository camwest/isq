use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::RwLock;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    AuthConfig, CreateGoalRequest, CreateIssueRequest, FetchResult, Forge, ForgeType, Goal, GoalState, Issue,
    Label, LinkArgs, LinkResult, RateLimitInfo,
};
use crate::repo::Repo;
use crate::{config, db, repo};

// ============================================================================
// Auth Configuration
// ============================================================================

/// JIRA authentication configuration
pub const AUTH: AuthConfig = AuthConfig {
    keyring_service: "jira",
    env_var: "JIRA_API_TOKEN",
    cli_command: None,
    display_name: "Jira",
    link_command: "isq link jira",
};

/// Default [on_start] config for JIRA repos
pub const DEFAULT_ON_START_TOML: &str = "transition = \"In Progress\"\nassign_self = true\n";

// ============================================================================
// API Configuration
// ============================================================================

// OAuth configuration
const JIRA_CLIENT_ID: &str = "VG2jV3YlB3mSWdHcLRZJ8kawl6BFWki8";
const JIRA_AUTH_URL: &str = "https://auth.atlassian.com/authorize";
const JIRA_RESOURCES_URL: &str = "https://api.atlassian.com/oauth/token/accessible-resources";
const REDIRECT_PORT: u16 = 19285;

// OAuth proxy service (handles token exchange with client_secret)
const SERVICE_URL: &str = "https://isq-jira-oauth.fly.dev";
const REDIRECT_URI: &str = "https://isq-jira-oauth.fly.dev/callback";

// OAuth scopes
// - read:jira-work, write:jira-work: issue operations
// - read:jira-user: /myself endpoint
// - manage:jira-project: version/goal operations
// - offline_access: refresh tokens
const JIRA_SCOPES: &str = "read:jira-work write:jira-work read:jira-user manage:jira-project offline_access";

// ============================================================================
// Priority Mapping
// ============================================================================

/// Truncate a string to max length with ellipsis (UTF-8 safe)
fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else if max_len <= 3 {
        // For very small max_len, just take first chars without ellipsis
        s.chars().take(max_len).collect()
    } else {
        let truncate_at = max_len - 3;
        let truncated: String = s.chars().take(truncate_at).collect();
        format!("{}...", truncated)
    }
}

/// Parse JIRA API error response and return a helpful error message
fn parse_jira_error(status: reqwest::StatusCode, body: &str) -> anyhow::Error {
    // Try to parse as JIRA error JSON: {"errorMessages":[], "errors":{"field":"message"}}
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let mut messages = Vec::new();

        // Collect error messages
        if let Some(error_messages) = json.get("errorMessages").and_then(|m| m.as_array()) {
            for msg in error_messages {
                if let Some(s) = msg.as_str() {
                    if !s.is_empty() {
                        messages.push(s.to_string());
                    }
                }
            }
        }

        // Collect field-specific errors with helpful hints
        if let Some(errors) = json.get("errors").and_then(|e| e.as_object()) {
            for (field, msg) in errors {
                if let Some(msg_str) = msg.as_str() {
                    let hint = match field.as_str() {
                        "issuetype" => " (hint: run `isq issue list -o jql=\"project=PROJ\" --json` to see valid issue types, or use -o type=Task)",
                        _ => "",
                    };
                    messages.push(format!("{}: {}{}", field, msg_str, hint));
                }
            }
        }

        if !messages.is_empty() {
            return anyhow!("JIRA error: {}", messages.join("; "));
        }
    }

    // Fallback to raw error
    anyhow!("JIRA API error ({}): {}", status, body)
}

/// Map JIRA priority name to our priority scale.
/// JIRA: Highest, High, Medium, Low, Lowest
/// Ours: 0=urgent, 1=high, 2=medium, 3=low, 4=none
fn map_jira_priority(priority_name: Option<&str>) -> u8 {
    match priority_name {
        Some("Highest") => 0,
        Some("High") => 1,
        Some("Medium") => 2,
        Some("Low") => 3,
        Some("Lowest") => 3,
        _ => 4, // unknown/none
    }
}

// ============================================================================
// JIRA-specific on_start configuration
// ============================================================================

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct JiraOnStartConfig {
    /// Workflow transition name (e.g., "In Progress", "Start Progress")
    transition: Option<String>,
    /// Assign the issue to yourself
    #[serde(default)]
    assign_self: bool,
}

// ============================================================================
// OAuth Flow
// ============================================================================

/// Token response from JIRA OAuth
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
}

/// Accessible resource (JIRA site) from Atlassian
#[derive(Debug, Deserialize)]
pub struct AccessibleResource {
    pub id: String,
    pub url: String,
    pub name: String,
    pub scopes: Vec<String>,
}

/// Generate a random code verifier for PKCE (43-128 chars, URL-safe)
fn generate_code_verifier() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen()).collect();
    URL_SAFE_NO_PAD.encode(&bytes)
}

/// Generate code challenge from verifier using S256 method
fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(&hash)
}

/// Build the authorization URL with PKCE
/// The code_verifier is encoded in the state parameter so the OAuth proxy service
/// can use it for token exchange (since we don't want to expose client_secret in CLI)
fn build_auth_url(code_challenge: &str, code_verifier: &str) -> String {
    // Encode code_verifier in state so service can use it for token exchange
    let state = URL_SAFE_NO_PAD.encode(code_verifier.as_bytes());
    format!(
        "{}?audience=api.atlassian.com&client_id={}&scope={}&redirect_uri={}&response_type=code&prompt=consent&code_challenge={}&code_challenge_method=S256&state={}",
        JIRA_AUTH_URL,
        JIRA_CLIENT_ID,
        urlencoding::encode(JIRA_SCOPES),
        urlencoding::encode(REDIRECT_URI),
        code_challenge,
        state
    )
}

/// Start a local server and wait for the OAuth callback from the proxy service
/// The service redirects here with either ?tokens=<base64> or ?error=<base64>
fn wait_for_callback() -> Result<TokenResponse> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", REDIRECT_PORT))
        .map_err(|e| anyhow!("Failed to start local server on port {}: {}", REDIRECT_PORT, e))?;

    listener.set_nonblocking(false)?;

    println!("Waiting for authorization...");

    for stream in listener.incoming() {
        let mut stream = stream?;
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;

        if let Some(path) = request_line.split_whitespace().nth(1) {
            if path.starts_with("/callback") {
                let query = path.strip_prefix("/callback?").unwrap_or("");
                let params: std::collections::HashMap<_, _> = query
                    .split('&')
                    .filter_map(|p| {
                        let mut parts = p.splitn(2, '=');
                        Some((parts.next()?, parts.next()?))
                    })
                    .collect();

                // Check for error from service
                if let Some(error_b64) = params.get("error") {
                    let error_bytes = URL_SAFE_NO_PAD.decode(error_b64).unwrap_or_default();
                    let error_msg = String::from_utf8(error_bytes).unwrap_or_else(|_| "Unknown error".to_string());
                    send_response(&mut stream, false, &format!("Authorization failed: {}", error_msg))?;
                    return Err(anyhow!("OAuth error: {}", error_msg));
                }

                // Get tokens from service
                let tokens_b64 = params
                    .get("tokens")
                    .ok_or_else(|| anyhow!("Missing tokens parameter"))?;

                let tokens_bytes = URL_SAFE_NO_PAD.decode(tokens_b64)
                    .map_err(|_| anyhow!("Failed to decode tokens"))?;
                let tokens_json = String::from_utf8(tokens_bytes)
                    .map_err(|_| anyhow!("Invalid tokens encoding"))?;
                let tokens: TokenResponse = serde_json::from_str(&tokens_json)
                    .map_err(|e| anyhow!("Failed to parse tokens: {}", e))?;

                send_response(
                    &mut stream,
                    true,
                    "Authorization successful! You can close this tab.",
                )?;
                return Ok(tokens);
            }
        }
    }

    Err(anyhow!("No callback received"))
}

/// Send HTTP response to browser
fn send_response(stream: &mut std::net::TcpStream, success: bool, message: &str) -> Result<()> {
    let (status, color) = if success {
        ("200 OK", "#22c55e")
    } else {
        ("400 Bad Request", "#ef4444")
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>isq</title></head>
<body style="font-family: system-ui; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #0a0a0a; color: #fafafa;">
<div style="text-align: center;">
<h1 style="color: {};">{}</h1>
<p style="color: #a1a1aa;">Return to your terminal.</p>
</div>
</body>
</html>"#,
        color, message
    );

    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        html.len(),
        html
    );

    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

/// Run the full OAuth flow for JIRA
/// Opens browser, waits for callback with tokens from OAuth proxy service
pub async fn oauth_flow() -> Result<TokenResponse> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);

    // code_verifier is encoded in state so service can use it for token exchange
    let auth_url = build_auth_url(&code_challenge, &code_verifier);

    println!("Opening browser to authorize...");
    open::that(&auth_url).map_err(|e| anyhow!("Failed to open browser: {}", e))?;

    // Service exchanges code for tokens and redirects here with tokens
    let token = wait_for_callback()?;

    Ok(token)
}

/// Refresh a JIRA access token using a refresh token
/// Uses the OAuth proxy service which has the client_secret
pub async fn refresh_token(refresh_token: &str) -> Result<TokenResponse> {
    let client = reqwest::Client::new();

    let refresh_url = format!("{}/refresh", SERVICE_URL);

    let response = client
        .post(&refresh_url)
        .json(&serde_json::json!({ "refresh_token": refresh_token }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(anyhow!("Token refresh failed ({}): {}", status, body));
    }

    let token: TokenResponse = response.json().await?;
    Ok(token)
}

/// Get accessible JIRA sites for the authenticated user
pub async fn get_accessible_resources(access_token: &str) -> Result<Vec<AccessibleResource>> {
    let client = reqwest::Client::new();

    let response = client
        .get(JIRA_RESOURCES_URL)
        .bearer_auth(access_token)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(anyhow!(
            "Failed to get accessible resources ({}): {}",
            status,
            body
        ));
    }

    let resources: Vec<AccessibleResource> = response.json().await?;
    Ok(resources)
}

// ============================================================================
// JIRA API Types
// ============================================================================

/// JIRA project from API
#[derive(Debug, Deserialize)]
pub struct JiraProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// JIRA issue type from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueType {
    pub id: String,
    pub name: String,
    pub subtask: bool,
}

/// JIRA user from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraUser {
    pub account_id: String,
    pub display_name: Option<String>,
    pub email_address: Option<String>,
}

/// JIRA status from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraStatus {
    pub id: String,
    pub name: String,
    pub status_category: JiraStatusCategory,
}

/// JIRA status category from API
#[derive(Debug, Deserialize)]
pub struct JiraStatusCategory {
    pub id: u64,
    pub key: String,
    pub name: String,
}

/// JIRA priority from API
#[derive(Debug, Deserialize)]
pub struct JiraPriority {
    pub id: String,
    pub name: String,
}

/// JIRA issue fields from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraIssueFields {
    pub summary: String,
    pub description: Option<serde_json::Value>, // ADF format
    pub status: JiraStatus,
    pub issuetype: JiraIssueType,
    pub priority: Option<JiraPriority>,
    pub reporter: Option<JiraUser>,
    pub assignee: Option<JiraUser>,
    pub labels: Option<Vec<String>>,
    pub created: String,
    pub updated: String,
    #[serde(rename = "fixVersions")]
    pub fix_versions: Option<Vec<JiraVersion>>,
}

/// JIRA issue from API
#[derive(Debug, Deserialize)]
pub struct JiraIssue {
    pub id: String,
    pub key: String,
    #[serde(rename = "self")]
    pub self_url: String,
    pub fields: JiraIssueFields,
}

/// JIRA create issue response (minimal - just id, key, self)
#[derive(Debug, Deserialize)]
pub struct JiraCreateResponse {
    pub id: String,
    pub key: String,
}

/// Minimal JIRA issue (for key-only queries)
#[derive(Debug, Deserialize)]
pub struct JiraIssueMinimal {
    pub key: String,
}

/// JIRA search response for minimal queries
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSearchResponseMinimal {
    pub issues: Vec<JiraIssueMinimal>,
    pub next_page_token: Option<String>,
}

/// JIRA search response (new /search/jql format)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraSearchResponse {
    pub issues: Vec<JiraIssue>,
    pub next_page_token: Option<String>,
}

/// JIRA version (for goals)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraVersion {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub released: Option<bool>,
    pub archived: Option<bool>,
    pub release_date: Option<String>,
}

/// JIRA transition from API
#[derive(Debug, Deserialize)]
pub struct JiraTransition {
    pub id: String,
    pub name: String,
}

/// JIRA transitions response
#[derive(Debug, Deserialize)]
pub struct JiraTransitionsResponse {
    pub transitions: Vec<JiraTransition>,
}

/// JIRA comment from API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraComment {
    pub id: String,
    pub body: Option<serde_json::Value>, // ADF format
    pub author: Option<JiraUser>,
    pub created: String,
    pub updated: String,
}

/// JIRA comments response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JiraCommentsResponse {
    pub comments: Vec<JiraComment>,
    pub total: u64,
    pub start_at: u64,
    pub max_results: u64,
}

// ============================================================================
// ADF Conversion
// ============================================================================

/// Convert Atlassian Document Format (ADF) to Markdown
pub fn adf_to_markdown(adf: &serde_json::Value) -> String {
    let mut output = String::new();
    if let Some(content) = adf.get("content").and_then(|c| c.as_array()) {
        for node in content {
            convert_adf_node(node, &mut output, 0);
        }
    }
    output.trim().to_string()
}

fn convert_adf_node(node: &serde_json::Value, output: &mut String, depth: usize) {
    let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match node_type {
        "paragraph" => {
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth);
                }
            }
            output.push_str("\n\n");
        }
        "heading" => {
            let level = node.get("attrs").and_then(|a| a.get("level")).and_then(|l| l.as_u64()).unwrap_or(1);
            output.push_str(&"#".repeat(level as usize));
            output.push(' ');
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth);
                }
            }
            output.push_str("\n\n");
        }
        "text" => {
            let text = node.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let marks = node.get("marks").and_then(|m| m.as_array());

            let mut prefix = String::new();
            let mut suffix = String::new();

            if let Some(marks) = marks {
                for mark in marks {
                    let mark_type = mark.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    match mark_type {
                        "strong" => { prefix.push_str("**"); suffix.insert_str(0, "**"); }
                        "em" => { prefix.push('*'); suffix.insert(0, '*'); }
                        "code" => { prefix.push('`'); suffix.insert(0, '`'); }
                        "strike" => { prefix.push_str("~~"); suffix.insert_str(0, "~~"); }
                        "link" => {
                            if let Some(href) = mark.get("attrs").and_then(|a| a.get("href")).and_then(|h| h.as_str()) {
                                prefix.push('[');
                                suffix = format!("]({}){}", href, suffix);
                            }
                        }
                        _ => {}
                    }
                }
            }

            output.push_str(&prefix);
            output.push_str(text);
            output.push_str(&suffix);
        }
        "hardBreak" => {
            output.push('\n');
        }
        "bulletList" => {
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth);
                }
            }
        }
        "orderedList" => {
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for (i, child) in content.iter().enumerate() {
                    // Store the index for numbered list items
                    output.push_str(&format!("{}. ", i + 1));
                    if let Some(item_content) = child.get("content").and_then(|c| c.as_array()) {
                        for item_child in item_content {
                            convert_adf_node(item_child, output, depth + 1);
                        }
                    }
                }
            }
        }
        "listItem" => {
            output.push_str("- ");
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth + 1);
                }
            }
        }
        "codeBlock" => {
            let language = node.get("attrs").and_then(|a| a.get("language")).and_then(|l| l.as_str()).unwrap_or("");
            output.push_str("```");
            output.push_str(language);
            output.push('\n');
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    if let Some(text) = child.get("text").and_then(|t| t.as_str()) {
                        output.push_str(text);
                    }
                }
            }
            output.push_str("\n```\n\n");
        }
        "blockquote" => {
            output.push_str("> ");
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth);
                }
            }
        }
        "rule" => {
            output.push_str("---\n\n");
        }
        "mention" => {
            let text = node.get("attrs").and_then(|a| a.get("text")).and_then(|t| t.as_str()).unwrap_or("@user");
            output.push_str(&format!("[{}]", text));
        }
        "mediaGroup" | "mediaSingle" => {
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth);
                }
            }
        }
        "media" => {
            let media_type = node.get("attrs").and_then(|a| a.get("type")).and_then(|t| t.as_str()).unwrap_or("file");
            let name = node.get("attrs").and_then(|a| a.get("alt")).and_then(|t| t.as_str())
                .or_else(|| node.get("attrs").and_then(|a| a.get("id")).and_then(|t| t.as_str()))
                .unwrap_or("attachment");
            output.push_str(&format!("[{}: {}]", media_type.to_uppercase(), name));
        }
        "emoji" => {
            let shortname = node.get("attrs").and_then(|a| a.get("shortName")).and_then(|t| t.as_str()).unwrap_or(":emoji:");
            output.push_str(shortname);
        }
        "table" => {
            output.push_str("[Table]\n\n");
        }
        _ => {
            // Unknown node type - try to recurse into content
            if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
                for child in content {
                    convert_adf_node(child, output, depth);
                }
            }
        }
    }
}

/// Convert Markdown to Atlassian Document Format (ADF)
/// Uses a simple approach - converts to paragraph nodes with text
pub fn markdown_to_adf(markdown: &str) -> serde_json::Value {
    // For now, create a simple ADF document with paragraphs
    // A full implementation would parse markdown properly
    let paragraphs: Vec<serde_json::Value> = markdown
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            serde_json::json!({
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": p.trim()
                }]
            })
        })
        .collect();

    serde_json::json!({
        "version": 1,
        "type": "doc",
        "content": paragraphs
    })
}

// ============================================================================
// JIRA Client
// ============================================================================

/// Authentication mode for JIRA
#[derive(Debug, Clone)]
pub enum JiraAuthMode {
    /// OAuth 2.0 - uses api.atlassian.com with cloud_id routing
    OAuth { cloud_id: String },
    /// API Token - uses basic auth directly to site
    ApiToken { email: String },
}

/// Stored JIRA credentials including site info
#[derive(Debug, Clone)]
pub struct JiraCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub auth_mode: JiraAuthMode,
    pub site_url: String,
    pub expires_at: Option<i64>,
}

/// JIRA API client
pub struct JiraClient {
    client: reqwest::Client,
    creds: RwLock<JiraCredentials>,
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
            JiraAuthMode::OAuth { .. } => {
                ("Bearer".to_string(), creds.access_token.clone())
            }
            JiraAuthMode::ApiToken { email } => {
                // Basic auth: base64(email:token)
                let basic = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", email, creds.access_token));
                ("Basic".to_string(), basic)
            }
        }
    }

    /// Get the site URL for building browse links
    fn site_url(&self) -> String {
        let creds = self.creds.read().unwrap();
        creds.site_url.clone()
    }

    /// Make an authenticated GET request
    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self.client.get(&url).header("Authorization", format!("{} {}", auth_type, auth_value)).send().await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "JIRA authentication failed. Please re-authenticate with: isq link jira"
            ));
        }

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!(
                "Access denied. You may not have permission to access this JIRA project."
            ));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(anyhow!("JIRA API error ({}): {}", status, body));
        }

        let result = response.json().await?;
        Ok(result)
    }

    /// Make an authenticated POST request
    async fn post<T: for<'de> Deserialize<'de>, B: Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!(
                "JIRA authentication failed. Please re-authenticate with: isq link jira"
            ));
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(parse_jira_error(status, &body));
        }

        let result = response.json().await?;
        Ok(result)
    }

    /// Make an authenticated POST request without expecting a response body
    async fn post_no_response<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            return Err(parse_jira_error(status, &body));
        }

        Ok(())
    }

    /// Make an authenticated PUT request
    async fn put<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        self.refresh_if_needed().await?;
        let url = format!("{}{}", self.api_base(), path);
        let (auth_type, auth_value) = self.auth_header();

        let response = self
            .client
            .put(&url)
            .header("Authorization", format!("{} {}", auth_type, auth_value))
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
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

        let expires_at = new_tokens.expires_in.map(|secs| {
            chrono::Utc::now().timestamp() + secs as i64
        });

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
            AUTH.store_credential(
                &cred_json.to_string(),
                None,
                None,
            )?;
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
        let path = format!("/mypermissions?projectKey={}&permissions=CREATE_ISSUES", project_key);

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
                let can_create = resp.permissions
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

    /// Parse issue number from either full key (PROJ-123) or just number (123)
    fn parse_issue_key(&self, repo: &Repo, issue_ref: &str) -> String {
        if issue_ref.contains('-') {
            // Already a full key
            issue_ref.to_string()
        } else {
            // Just a number - prepend project key (repo.name is the project key)
            format!("{}-{}", repo.name, issue_ref)
        }
    }

    /// List available JIRA fields (for JQL queries)
    pub async fn list_fields(&self) -> Result<()> {
        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct JiraField {
            id: String,
            name: String,
            #[serde(default)]
            searchable: bool,
            #[serde(default)]
            clause_names: Vec<String>,
            schema: Option<JiraFieldSchema>,
        }

        #[derive(Debug, Deserialize)]
        struct JiraFieldSchema {
            #[serde(rename = "type")]
            field_type: Option<String>,
        }

        let fields: Vec<JiraField> = self.get("/field").await?;

        // Filter to searchable fields and sort by name
        let mut searchable: Vec<_> = fields.iter().filter(|f| f.searchable).collect();
        searchable.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        println!("{:<30} {:<25} {:<15} {}", "Name", "JQL Clause", "Type", "ID");
        println!("{}", "-".repeat(85));

        for field in &searchable {
            let clause = field.clause_names.first().map(|s| s.as_str()).unwrap_or(&field.id);
            let field_type = field.schema.as_ref()
                .and_then(|s| s.field_type.as_deref())
                .unwrap_or("unknown");
            println!("{:<30} {:<25} {:<15} {}",
                truncate(&field.name, 29),
                truncate(clause, 24),
                truncate(field_type, 14),
                &field.id
            );
        }

        println!("\n{} searchable fields", searchable.len());
        println!("\nExample JQL queries:");
        println!("  isq issue list -o jql=\"assignee = currentUser()\"");
        println!("  isq issue list -o jql=\"priority = High\"");
        println!("  isq issue list -o jql=\"status = 'In Progress'\"");

        Ok(())
    }

    /// Convert a JIRA issue to our Issue type
    fn convert_issue(&self, jira_issue: &JiraIssue) -> Issue {
        let state = match jira_issue.fields.status.status_category.key.as_str() {
            "done" => "closed",
            _ => "open",
        };

        let mut labels: Vec<Label> = jira_issue
            .fields
            .labels
            .as_ref()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|l| Label::name_only(l.clone()))
            .collect();

        // Add issue type as pseudo-label
        labels.push(Label::name_only(format!(
            "type:{}",
            jira_issue.fields.issuetype.name
        )));

        let priority = map_jira_priority(
            jira_issue
                .fields
                .priority
                .as_ref()
                .map(|p| p.name.as_str()),
        );

        let assignees: Vec<String> = jira_issue
            .fields
            .assignee
            .as_ref()
            .map(|a| {
                a.display_name
                    .as_ref()
                    .unwrap_or(&a.account_id)
                    .clone()
            })
            .into_iter()
            .collect();

        let body = jira_issue
            .fields
            .description
            .as_ref()
            .map(|d| adf_to_markdown(d));

        let milestone = jira_issue
            .fields
            .fix_versions
            .as_ref()
            .and_then(|v| v.first())
            .map(|v| v.name.clone());

        // Extract issue number from key (e.g., "PROJ-123" -> 123)
        let number: u64 = jira_issue
            .key
            .split('-')
            .last()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);

        Issue {
            number,
            key: Some(jira_issue.key.clone()),
            title: jira_issue.fields.summary.clone(),
            body,
            state: state.to_string(),
            author: jira_issue
                .fields
                .reporter
                .as_ref()
                .and_then(|r| r.display_name.clone())
                .unwrap_or_default(),
            labels,
            assignees,
            priority,
            priority_label: None,
            created_at: jira_issue.fields.created.clone(),
            updated_at: jira_issue.fields.updated.clone(),
            url: Some(format!("{}/browse/{}", self.site_url(), jira_issue.key)),
            milestone,
        }
    }
/// Internal list_issues with optional since filter for incremental sync
    async fn list_issues_internal(&self, repo: &Repo, since: Option<DateTime<Utc>>) -> Result<FetchResult<Issue>> {
        let project_key = &repo.name;

        let mut all_issues = Vec::new();
        let mut next_page_token: Option<String> = None;
        let mut page = 0;

        loop {
            // Build JQL with optional updated filter for incremental sync
            let jql = match since {
                Some(ts) => format!(
                    "project = {} AND updated >= \"{}\" ORDER BY updated ASC",
                    project_key,
                    ts.format("%Y-%m-%d %H:%M")
                ),
                None => format!("project = {} ORDER BY updated DESC", project_key),
            };

            let body = serde_json::json!({
                "jql": jql,
                "maxResults": 100,
                "fields": ["*all"],
                "nextPageToken": next_page_token
            });

            let response: JiraSearchResponse = self.post("/search/jql", &body).await?;

            for jira_issue in &response.issues {
                all_issues.push(self.convert_issue(jira_issue));
            }

            page += 1;
            // Print progress every 10 pages
            if page % 10 == 0 {
                eprintln!("  {} issues...", all_issues.len());
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(FetchResult::complete(all_issues))
    }

    /// Internal list_all_comments with optional since filter for incremental sync
    async fn list_all_comments_internal(&self, repo: &Repo, since: Option<DateTime<Utc>>) -> Result<FetchResult<db::Comment>> {
        let project_key = &repo.name;

        let mut all_comments = Vec::new();
        let mut next_page_token: Option<String> = None;
        let mut page = 0;

        loop {
            // Build JQL with optional updated filter for incremental sync
            let jql = match since {
                Some(ts) => format!(
                    "project = {} AND updated >= \"{}\" ORDER BY updated ASC",
                    project_key,
                    ts.format("%Y-%m-%d %H:%M")
                ),
                None => format!("project = {} ORDER BY updated DESC", project_key),
            };

            let body = serde_json::json!({
                "jql": jql,
                "maxResults": 100,
                "fields": ["key"],
                "nextPageToken": next_page_token
            });

            let response: JiraSearchResponseMinimal = self.post("/search/jql", &body).await?;

            // For each issue, fetch comments
            for jira_issue in &response.issues {
                let issue_number: u64 = jira_issue
                    .key
                    .split('-')
                    .last()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);

                // Paginate through all comments for this issue
                let mut start_at: u64 = 0;
                loop {
                    let comments_path = format!(
                        "/issue/{}/comment?startAt={}&maxResults=100",
                        jira_issue.key, start_at
                    );
                    let comments_response: JiraCommentsResponse =
                        match self.get(&comments_path).await {
                            Ok(r) => r,
                            Err(_) => break, // Skip issues we can't read comments from
                        };

                    for comment in &comments_response.comments {
                        let body = comment
                            .body
                            .as_ref()
                            .map(|b| adf_to_markdown(b))
                            .unwrap_or_default();

                        all_comments.push(db::Comment {
                            comment_id: comment.id.clone(),
                            issue_number,
                            body,
                            author: comment
                                .author
                                .as_ref()
                                .and_then(|a| a.display_name.clone())
                                .unwrap_or_default(),
                            created_at: comment.created.clone(),
                            updated_at: Some(comment.updated.clone()),
                        });
                    }

                    // Check if there are more pages
                    // Break if no comments returned (prevents infinite loop on restricted comments)
                    if comments_response.comments.is_empty() {
                        break;
                    }
                    let fetched = start_at + comments_response.comments.len() as u64;
                    if fetched >= comments_response.total {
                        break;
                    }
                    start_at = fetched;
                }
            }

            page += 1;
            // Print progress every 10 pages
            if page % 10 == 0 {
                eprintln!("  {} comments...", all_comments.len());
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(FetchResult::complete(all_comments))
    }
}

// ============================================================================
// Forge Trait Implementation
// ============================================================================

#[async_trait]
impl Forge for JiraClient {
    async fn list_issues(&self, repo: &Repo) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, None).await
    }

    async fn list_issues_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<FetchResult<Issue>> {
        self.list_issues_internal(repo, Some(since)).await
    }

    async fn create_issue(&self, repo: &Repo, req: CreateIssueRequest) -> Result<Issue> {
        let project_key = &repo.name;

        // Get issue type from opts, or default to "Task"
        let issue_type = req.opts.get("type").map(|s| s.as_str()).unwrap_or("Task");

        let description_adf = req.body.as_ref().map(|b| markdown_to_adf(b));

        let mut fields = serde_json::json!({
            "project": { "key": project_key },
            "summary": req.title,
            "issuetype": { "name": issue_type }
        });

        if let Some(desc) = description_adf {
            fields["description"] = desc;
        }

        if !req.labels.is_empty() {
            fields["labels"] = serde_json::json!(req.labels);
        }

        // TODO: handle goal_id -> fixVersions mapping

        let body = serde_json::json!({ "fields": fields });
        let created: JiraCreateResponse = self.post("/issue", &body).await?;

        // Fetch full issue to get all fields
        let path = format!("/issue/{}", created.key);
        let full_issue: JiraIssue = self.get(&path).await?;

        Ok(self.convert_issue(&full_issue))
    }

    async fn create_comment(&self, repo: &Repo, issue_number: u64, body: &str) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());
        let path = format!("/issue/{}/comment", issue_key);

        let comment_body = serde_json::json!({
            "body": markdown_to_adf(body)
        });

        self.post_no_response(&path, &comment_body).await
    }

    async fn close_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());

        // Get available transitions
        let path = format!("/issue/{}/transitions", issue_key);
        let response: JiraTransitionsResponse = self.get(&path).await?;

        // Find a "Done" transition
        let done_transition = response
            .transitions
            .iter()
            .find(|t| {
                let name_lower = t.name.to_lowercase();
                name_lower == "done" || name_lower.contains("done") || name_lower.contains("close")
            })
            .ok_or_else(|| anyhow!("No 'Done' transition available for this issue"))?;

        let body = serde_json::json!({
            "transition": { "id": done_transition.id }
        });

        self.post_no_response(&path, &body).await
    }

    async fn reopen_issue(&self, repo: &Repo, issue_number: u64) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());

        // Get available transitions
        let path = format!("/issue/{}/transitions", issue_key);
        let response: JiraTransitionsResponse = self.get(&path).await?;

        // Find a "To Do" or "Reopen" transition
        let reopen_transition = response
            .transitions
            .iter()
            .find(|t| {
                let name_lower = t.name.to_lowercase();
                name_lower == "to do"
                    || name_lower.contains("reopen")
                    || name_lower.contains("backlog")
            })
            .ok_or_else(|| anyhow!("No 'Reopen' transition available for this issue"))?;

        let body = serde_json::json!({
            "transition": { "id": reopen_transition.id }
        });

        self.post_no_response(&path, &body).await
    }

    async fn add_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());
        let path = format!("/issue/{}", issue_key);

        let body = serde_json::json!({
            "update": {
                "labels": [{ "add": label }]
            }
        });

        self.put(&path, &body).await
    }

    async fn remove_label(&self, repo: &Repo, issue_number: u64, label: &str) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());
        let path = format!("/issue/{}", issue_key);

        let body = serde_json::json!({
            "update": {
                "labels": [{ "remove": label }]
            }
        });

        self.put(&path, &body).await
    }

    async fn assign_issue(&self, repo: &Repo, issue_number: u64, assignee: &str) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());
        let path = format!("/issue/{}/assignee", issue_key);

        // Handle unassign case
        let body = if assignee.is_empty() || assignee == "null" {
            serde_json::json!({ "accountId": null })
        } else {
            // assignee should be an account ID
            serde_json::json!({ "accountId": assignee })
        };

        self.put(&path, &body).await
    }

    async fn list_all_comments(&self, repo: &Repo) -> Result<FetchResult<db::Comment>> {
        self.list_all_comments_internal(repo, None).await
    }

    async fn list_comments_since(&self, repo: &Repo, since: DateTime<Utc>) -> Result<FetchResult<db::Comment>> {
        self.list_all_comments_internal(repo, Some(since)).await
    }

    async fn list_goals(&self, repo: &Repo) -> Result<Vec<Goal>> {
        let project_key = &repo.name;
        let path = format!("/project/{}/versions", project_key);

        let versions: Vec<JiraVersion> = self.get(&path).await?;

        let goals: Vec<Goal> = versions
            .into_iter()
            .map(|v| {
                let state = if v.released.unwrap_or(false) || v.archived.unwrap_or(false) {
                    GoalState::Closed
                } else {
                    GoalState::Open
                };

                Goal {
                    id: v.id,
                    name: v.name,
                    description: v.description,
                    target_date: v.release_date,
                    state,
                    progress: 0.0, // TODO: calculate from issues
                    open_count: None,
                    closed_count: None,
                    created_at: String::new(), // Versions don't have created_at
                    updated_at: String::new(),
                    html_url: None,
                }
            })
            .collect();

        Ok(goals)
    }

    async fn create_goal(&self, repo: &Repo, req: CreateGoalRequest) -> Result<Goal> {
        let project_key = &repo.name;

        // First, get project ID from key
        let project_path = format!("/project/{}", project_key);
        let project: JiraProject = self.get(&project_path).await?;

        let project_id: i64 = project.id.parse().map_err(|_| {
            anyhow!("Invalid project ID '{}' - expected numeric value", project.id)
        })?;

        let body = serde_json::json!({
            "name": req.name,
            "description": req.description,
            "releaseDate": req.target_date,
            "projectId": project_id
        });

        let version: JiraVersion = self.post("/version", &body).await?;

        Ok(Goal {
            id: version.id,
            name: version.name,
            description: version.description,
            target_date: version.release_date,
            state: GoalState::Open,
            progress: 0.0,
            open_count: None,
            closed_count: None,
            created_at: String::new(),
            updated_at: String::new(),
            html_url: None,
        })
    }

    async fn close_goal(&self, _repo: &Repo, goal_id: &str) -> Result<()> {
        let path = format!("/version/{}", goal_id);
        let body = serde_json::json!({ "released": true });
        self.put(&path, &body).await
    }

    async fn assign_to_goal(&self, repo: &Repo, issue_number: u64, goal_id: &str) -> Result<()> {
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());
        let path = format!("/issue/{}", issue_key);

        // Get version name from ID
        let version_path = format!("/version/{}", goal_id);
        let version: JiraVersion = self.get(&version_path).await?;

        let body = serde_json::json!({
            "update": {
                "fixVersions": [{ "add": { "name": version.name } }]
            }
        });

        self.put(&path, &body).await
    }

    async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
        // JIRA doesn't expose rate limit info in a simple endpoint
        // We'd need to track it from response headers
        Ok(None)
    }

    async fn list_labels(&self, _repo: &Repo) -> Result<Vec<Label>> {
        // JIRA labels are freeform and not stored separately
        // We could get them from issues, but for now return empty
        Ok(Vec::new())
    }

    async fn create_label(
        &self,
        _repo: &Repo,
        name: &str,
        _color: Option<&str>,
        _description: Option<&str>,
    ) -> Result<Label> {
        // JIRA labels are created implicitly when added to issues
        Ok(Label::name_only(name.to_string()))
    }

    async fn handle_on_start(
        &self,
        repo: &Repo,
        issue_number: u64,
        config: &toml::Value,
        username: Option<&str>,
    ) -> Result<()> {
        let on_start: JiraOnStartConfig = config.clone().try_into()?;
        let issue_key = self.parse_issue_key(repo, &issue_number.to_string());

        // Handle transition
        if let Some(transition_name) = &on_start.transition {
            let path = format!("/issue/{}/transitions", issue_key);
            let response: JiraTransitionsResponse = self.get(&path).await?;

            let transition = response
                .transitions
                .iter()
                .find(|t| t.name.to_lowercase() == transition_name.to_lowercase())
                .ok_or_else(|| {
                    let available: Vec<_> = response.transitions.iter().map(|t| &t.name).collect();
                    anyhow!(
                        "Transition '{}' not available. Available transitions: {:?}",
                        transition_name,
                        available
                    )
                })?;

            let body = serde_json::json!({
                "transition": { "id": transition.id }
            });

            self.post_no_response(&path, &body).await?;
        }

        // Handle assign_self
        if on_start.assign_self {
            if let Some(account_id) = username {
                self.assign_issue(repo, issue_number, account_id).await?;
            }
        }

        Ok(())
    }

    fn validate_on_start_config(&self, config: &toml::Value) -> Result<()> {
        let _: JiraOnStartConfig = config.clone().try_into().context(
            "Invalid [on_start] config for JIRA. Expected: transition = \"In Progress\", assign_self = true",
        )?;
        Ok(())
    }

    async fn handle_command(&self, command: &str, _args: &[String]) -> Result<()> {
        match command {
            "list-fields" => self.list_fields().await,
            _ => Err(anyhow!("Unknown command: {}. Available commands: list-fields", command)),
        }
    }

    async fn query_issues_with_opts(
        &self,
        repo: &Repo,
        opts: &std::collections::HashMap<String, String>,
    ) -> Result<Option<Vec<Issue>>> {
        let jql_opt = opts.get("jql");
        let type_opt = opts.get("type");

        // If no JIRA-specific options, use cache
        if jql_opt.is_none() && type_opt.is_none() {
            return Ok(None);
        }

        let project_key = &repo.name;

        // Build JQL from options
        let mut conditions = vec![format!("project = {}", project_key)];

        if let Some(jql) = jql_opt {
            conditions.push(format!("({})", jql));
        }

        if let Some(issue_type) = type_opt {
            conditions.push(format!("issuetype = \"{}\"", issue_type));
        }

        let full_jql = conditions.join(" AND ");

        let mut all_issues = Vec::new();
        let mut next_page_token: Option<String> = None;

        loop {
            let body = serde_json::json!({
                "jql": full_jql,
                "maxResults": 100,
                "fields": ["*all"],
                "nextPageToken": next_page_token
            });

            let response: JiraSearchResponse = self.post("/search/jql", &body).await?;

            for jira_issue in &response.issues {
                all_issues.push(self.convert_issue(jira_issue));
            }

            match response.next_page_token {
                Some(token) => next_page_token = Some(token),
                None => break,
            }
        }

        Ok(Some(all_issues))
    }
}

// ============================================================================
// Link Flow
// ============================================================================

/// Run the complete JIRA link flow.
/// Handles auth, site selection, project selection, syncs issues, and returns the result.
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult> {
    let forge_type = ForgeType::Jira;
    let conn = db::open()?;

    // Try auth in order: env var (for CI) -> keyring -> OAuth
    let creds = if let Ok(env_creds) = get_credentials_from_env() {
        println!("Using JIRA credentials from JIRA_API_TOKEN");
        env_creds
    } else if let Ok(stored_creds) = get_stored_credentials() {
        println!("Using existing JIRA credentials");
        stored_creds
    } else {
        // Run OAuth flow
        let token = oauth_flow().await?;

        // Get accessible sites
        let sites = get_accessible_resources(&token.access_token).await?;
        if sites.is_empty() {
            anyhow::bail!("No JIRA sites accessible with this account");
        }

        // Select site (auto if one, otherwise require -o site=X)
        let site = if sites.len() == 1 {
            println!("Using site: {}", sites[0].name);
            &sites[0]
        } else {
            // Check for site argument
            if let Some(site_name) = args.get("site") {
                sites
                    .iter()
                    .find(|s| {
                        s.name.to_lowercase() == site_name.to_lowercase()
                            || s.url.contains(site_name)
                    })
                    .ok_or_else(|| {
                        let available: Vec<_> = sites.iter().map(|s| &s.name).collect();
                        anyhow!(
                            "Site '{}' not found. Available sites: {:?}",
                            site_name,
                            available
                        )
                    })?
            } else {
                let available: Vec<_> = sites.iter().map(|s| &s.name).collect();
                anyhow::bail!(
                    "Multiple JIRA sites available. Specify one with -o site=<name>.\n\nAvailable sites: {:?}",
                    available
                );
            }
        };

        let expires_at = token
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs as i64);

        let creds = JiraCredentials {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            auth_mode: JiraAuthMode::OAuth { cloud_id: site.id.clone() },
            site_url: site.url.clone(),
            expires_at,
        };

        // Store credentials
        store_credentials(&creds)?;

        creds
    };

    let client = JiraClient::new(creds.clone());

    // List available projects
    let projects = client.list_projects().await?;
    if projects.is_empty() {
        anyhow::bail!("No projects found in this JIRA site");
    }

    // Handle -o list-projects flag
    if args.has_flag("list-projects") {
        println!("Available projects:");
        for project in &projects {
            println!("  {} - {}", project.key, project.name);
        }
        return Err(anyhow!("-o list-projects: showing available projects"));
    }

    // Resolve project from -o project=X argument or auto-select if only one
    let project = if let Some(project_query) = args.get("project") {
        let query_lower = project_query.to_lowercase();
        projects
            .iter()
            .find(|p| p.key.to_lowercase() == query_lower || p.name.to_lowercase() == query_lower)
            .ok_or_else(|| {
                let available: Vec<_> = projects
                    .iter()
                    .map(|p| format!("{} ({})", p.key, p.name))
                    .collect();
                anyhow!(
                    "Project '{}' not found.\n\nAvailable projects:\n  {}",
                    project_query,
                    available.join("\n  ")
                )
            })?
    } else if projects.len() == 1 {
        println!("Using project: {} ({})", projects[0].key, projects[0].name);
        &projects[0]
    } else {
        let available: Vec<_> = projects
            .iter()
            .map(|p| format!("{} ({})", p.key, p.name))
            .collect();
        anyhow::bail!(
            "Multiple projects available. Specify one with -o project=<key>.\n\nAvailable projects:\n  {}\n\nExample: isq link jira -o project=\"{}\"",
            available.join("\n  "),
            projects[0].key
        );
    };

    // Check write permissions
    if !client.check_write_permission(&project.key).await? {
        anyhow::bail!(
            "You don't have write access to project {}. isq requires write permissions to function properly.",
            project.key
        );
    }

    // Get current user for display
    let user = client.get_current_user().await?;
    let display_name = user.display_name.unwrap_or_else(|| user.account_id.clone());

    // Create repo identifier: site/project_key
    let site_host = creds
        .site_url
        .replace("https://", "")
        .replace("http://", "");
    let forge_repo = format!("{}/{}", site_host, project.key);

    // Create pseudo-repo for syncing (JIRA uses site_host as owner, project_key as name)
    let pseudo_repo = repo::Repo {
        owner: site_host.clone(),
        name: project.key.clone(),
    };

    // Sync issues
    println!("Syncing issues from {}...", project.key);
    let issues = client.list_issues(&pseudo_repo).await?;

    // Save to database
    let full_display_name = format!("{} ({})", project.name, display_name);
    db::set_repo_link(&conn, repo_path, forge_type.as_str(), &forge_repo, Some(&full_display_name), Some(&user.account_id), Some(&display_name))?;
    db::save_issues(&conn, &forge_repo, &issues.items, true, true)?;
    db::add_watched_repo(&conn, repo_path)?;

    // Create .config/isq.toml with defaults
    if config::create_repo_config(std::path::Path::new(repo_path), forge_type.as_str())? {
        println!("✓ Created .config/isq.toml");
    }

    // Install commit hook
    match repo::install_hook(std::path::Path::new(repo_path)) {
        Ok(true) => println!("✓ Installed commit hook"),
        Ok(false) => {} // Already installed, silent
        Err(e) => eprintln!("Warning: Could not install hook: {}", e),
    }

    println!("✓ Synced {} issues", issues.items.len());

    // Sync goals
    let goals = client.list_goals(&pseudo_repo).await?;
    db::save_goals(&conn, &forge_repo, &goals)?;
    if !goals.is_empty() {
        println!("✓ Synced {} versions", goals.len());
    }

    Ok(LinkResult {
        display_name: full_display_name,
    })
}

/// Get credentials for a specific repo (used by get_forge_for_repo)
pub fn get_credentials_for_repo(_repo_id: &str) -> Result<JiraCredentials> {
    // Try stored credentials first (OAuth flow), then fall back to env var (API token)
    if let Ok(creds) = get_stored_credentials() {
        return Ok(creds);
    }
    if let Ok(creds) = get_credentials_from_env() {
        return Ok(creds);
    }
    Err(anyhow!("No JIRA credentials found. Run 'isq link jira' or set JIRA_API_TOKEN"))
}

/// Get stored JIRA credentials from keyring (OAuth flow)
fn get_stored_credentials() -> Result<JiraCredentials> {
    let cred = AUTH
        .get_credential()?
        .ok_or_else(|| anyhow!("No stored credentials"))?;

    // Parse JSON-encoded credentials
    let json: serde_json::Value = serde_json::from_str(&cred.access_token)
        .map_err(|_| anyhow!("Invalid stored credentials format"))?;

    let cloud_id = json["cloud_id"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing cloud_id"))?
        .to_string();

    Ok(JiraCredentials {
        access_token: json["access_token"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing access_token"))?
            .to_string(),
        refresh_token: json["refresh_token"].as_str().map(|s| s.to_string()),
        auth_mode: JiraAuthMode::OAuth { cloud_id },
        site_url: json["site_url"]
            .as_str()
            .ok_or_else(|| anyhow!("Missing site_url"))?
            .to_string(),
        expires_at: json["expires_at"].as_i64(),
    })
}

/// Store JIRA credentials in keyring (only for OAuth mode)
fn store_credentials(creds: &JiraCredentials) -> Result<()> {
    // Only store OAuth credentials - API tokens come from env var
    let cloud_id = match &creds.auth_mode {
        JiraAuthMode::OAuth { cloud_id } => cloud_id.clone(),
        JiraAuthMode::ApiToken { .. } => {
            // Don't store API token credentials in keyring
            return Ok(());
        }
    };

    let cred_json = serde_json::json!({
        "access_token": creds.access_token,
        "refresh_token": creds.refresh_token,
        "cloud_id": cloud_id,
        "site_url": creds.site_url,
        "expires_at": creds.expires_at
    });

    AUTH.store_credential(&cred_json.to_string(), None, None)
}

/// Try to get credentials from JIRA_API_TOKEN env var (for headless/CI use)
/// Format: {"email":"user@acme.com","token":"abc123","site":"acme.atlassian.net"}
fn get_credentials_from_env() -> Result<JiraCredentials> {
    let env_val = std::env::var(AUTH.env_var)
        .map_err(|_| anyhow!("JIRA_API_TOKEN not set"))?;

    let json: serde_json::Value = serde_json::from_str(&env_val)
        .map_err(|_| anyhow!("JIRA_API_TOKEN must be JSON: {{\"email\":\"...\",\"token\":\"...\",\"site\":\"...\"}}"))?;

    let email = json["email"]
        .as_str()
        .ok_or_else(|| anyhow!("JIRA_API_TOKEN missing 'email' field"))?
        .to_string();
    let token = json["token"]
        .as_str()
        .ok_or_else(|| anyhow!("JIRA_API_TOKEN missing 'token' field"))?;
    let site = json["site"]
        .as_str()
        .ok_or_else(|| anyhow!("JIRA_API_TOKEN missing 'site' field"))?;

    // Build site URL
    let site_url = if site.starts_with("http") {
        site.to_string()
    } else {
        format!("https://{}", site)
    };

    // For API token auth, access_token stores the raw token (not base64)
    // The auth_header() method will encode it when needed
    Ok(JiraCredentials {
        access_token: token.to_string(),
        refresh_token: None,
        auth_mode: JiraAuthMode::ApiToken { email },
        site_url,
        expires_at: None, // API tokens don't expire
    })
}

// Simple URL encoding implementation
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut result = String::new();
        for byte in s.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        result
    }
}
