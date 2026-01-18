//! OAuth PKCE flow for JIRA authentication

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{
    AUTH, JIRA_AUTH_URL, JIRA_CLIENT_ID, JIRA_RESOURCES_URL, JIRA_SCOPES, REDIRECT_PORT,
    REDIRECT_URI, SERVICE_URL, urlencoding,
};

/// Token response from JIRA OAuth
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
    #[allow(dead_code)]
    pub scope: Option<String>,
}

/// Accessible resource (JIRA site) from Atlassian
#[derive(Debug, Deserialize)]
pub struct AccessibleResource {
    pub id: String,
    pub url: String,
    pub name: String,
    #[allow(dead_code)]
    pub scopes: Vec<String>,
}

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

/// Generate a random code verifier for PKCE (43-128 chars, URL-safe)
fn generate_code_verifier() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen::<u8>()).collect();
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
    let listener = TcpListener::bind(format!("127.0.0.1:{}", REDIRECT_PORT)).map_err(|e| {
        anyhow!(
            "Failed to start local server on port {}: {}",
            REDIRECT_PORT,
            e
        )
    })?;

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
                    let error_msg = String::from_utf8(error_bytes)
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    send_response(
                        &mut stream,
                        false,
                        &format!("Authorization failed: {}", error_msg),
                    )?;
                    return Err(anyhow!("OAuth error: {}", error_msg));
                }

                // Get tokens from service
                let tokens_b64 = params
                    .get("tokens")
                    .ok_or_else(|| anyhow!("Missing tokens parameter"))?;

                let tokens_bytes = URL_SAFE_NO_PAD
                    .decode(tokens_b64)
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

/// Get stored JIRA credentials from keyring (OAuth flow)
pub fn get_stored_credentials() -> Result<JiraCredentials> {
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
pub fn store_credentials(creds: &JiraCredentials) -> Result<()> {
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
pub fn get_credentials_from_env() -> Result<JiraCredentials> {
    let env_val = std::env::var(AUTH.env_var).map_err(|_| anyhow!("JIRA_API_TOKEN not set"))?;

    let json: serde_json::Value = serde_json::from_str(&env_val).map_err(|_| {
        anyhow!(
            "JIRA_API_TOKEN must be JSON: {{\"email\":\"...\",\"token\":\"...\",\"site\":\"...\"}}"
        )
    })?;

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
