//! OAuth PKCE flow for Linear authentication

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use sha2::{Digest, Sha256};

use super::{urlencoding, LINEAR_CLIENT_ID, LINEAR_AUTH_URL, LINEAR_TOKEN_URL, REDIRECT_PORT, REDIRECT_URI};
use crate::forges::create_http_client;

/// Token response from Linear OAuth
#[derive(serde::Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: Option<u64>,
    pub refresh_token: Option<String>,
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
fn build_auth_url(code_challenge: &str, state: &str) -> String {
    format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        LINEAR_AUTH_URL,
        LINEAR_CLIENT_ID,
        urlencoding::encode(REDIRECT_URI),
        urlencoding::encode("read,write"),
        code_challenge,
        state
    )
}

/// Start a local server and wait for the OAuth callback
fn wait_for_callback(expected_state: &str) -> Result<String> {
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

                if let Some(error) = params.get("error") {
                    let description = params.get("error_description").unwrap_or(&"Unknown error");
                    send_response(&mut stream, false, &format!("Authorization failed: {}", description))?;
                    return Err(anyhow!("OAuth error: {} - {}", error, description));
                }

                let state = params.get("state").ok_or_else(|| anyhow!("Missing state parameter"))?;
                if *state != expected_state {
                    send_response(&mut stream, false, "State mismatch - possible CSRF attack")?;
                    return Err(anyhow!("State mismatch"));
                }

                let code = params.get("code").ok_or_else(|| anyhow!("Missing code parameter"))?;
                send_response(&mut stream, true, "Authorization successful! You can close this tab.")?;
                return Ok(code.to_string());
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
        color,
        message
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

/// Exchange authorization code for access token
async fn exchange_code(code: &str, code_verifier: &str) -> Result<TokenResponse> {
    let client = create_http_client();

    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", LINEAR_CLIENT_ID),
        ("redirect_uri", REDIRECT_URI),
        ("code", code),
        ("code_verifier", code_verifier),
    ];

    let response = client
        .post(LINEAR_TOKEN_URL)
        .form(&params)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        return Err(anyhow!("Token exchange failed ({}): {}", status, body));
    }

    let token: TokenResponse = response.json().await?;
    Ok(token)
}

/// Run the full OAuth flow for Linear
/// Opens browser, waits for callback, exchanges code for token
pub async fn oauth_flow() -> Result<TokenResponse> {
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = generate_code_verifier();

    let auth_url = build_auth_url(&code_challenge, &state);

    println!("Opening browser to authorize...");
    open::that(&auth_url).map_err(|e| anyhow!("Failed to open browser: {}", e))?;

    let code = wait_for_callback(&state)?;

    println!("Exchanging authorization code...");
    let token = exchange_code(&code, &code_verifier).await?;

    Ok(token)
}

/// Refresh a Linear access token using a refresh token
pub async fn refresh_token(refresh_token: &str) -> Result<TokenResponse> {
    let client = create_http_client();

    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", LINEAR_CLIENT_ID),
        ("refresh_token", refresh_token),
    ];

    let response = client
        .post(LINEAR_TOKEN_URL)
        .form(&params)
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
