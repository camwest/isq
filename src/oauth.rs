use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

/// Linear OAuth configuration
const LINEAR_CLIENT_ID: &str = "a6c010f01947bd3b847cb3c1707366e5";
const LINEAR_AUTH_URL: &str = "https://linear.app/oauth/authorize";
const LINEAR_TOKEN_URL: &str = "https://api.linear.app/oauth/token";
const REDIRECT_PORT: u16 = 19284;
const REDIRECT_URI: &str = "http://127.0.0.1:19284/callback";

/// Token response from Linear OAuth
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<u64>,
    pub scope: Option<String>,
    pub refresh_token: Option<String>,
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

    // Set a timeout so we don't hang forever
    listener.set_nonblocking(false)?;

    println!("Waiting for authorization...");

    for stream in listener.incoming() {
        let mut stream = stream?;
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;

        // Parse the request line: GET /callback?code=xxx&state=yyy HTTP/1.1
        if let Some(path) = request_line.split_whitespace().nth(1) {
            if path.starts_with("/callback") {
                // Parse query parameters
                let query = path.strip_prefix("/callback?").unwrap_or("");
                let params: std::collections::HashMap<_, _> = query
                    .split('&')
                    .filter_map(|p| {
                        let mut parts = p.splitn(2, '=');
                        Some((parts.next()?, parts.next()?))
                    })
                    .collect();

                // Check for error
                if let Some(error) = params.get("error") {
                    let description = params.get("error_description").unwrap_or(&"Unknown error");
                    send_response(&mut stream, false, &format!("Authorization failed: {}", description))?;
                    return Err(anyhow!("OAuth error: {} - {}", error, description));
                }

                // Verify state
                let state = params.get("state").ok_or_else(|| anyhow!("Missing state parameter"))?;
                if *state != expected_state {
                    send_response(&mut stream, false, "State mismatch - possible CSRF attack")?;
                    return Err(anyhow!("State mismatch"));
                }

                // Get authorization code
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
    let client = reqwest::Client::new();

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
pub async fn linear_oauth_flow() -> Result<TokenResponse> {
    // Generate PKCE values
    let code_verifier = generate_code_verifier();
    let code_challenge = generate_code_challenge(&code_verifier);
    let state = generate_code_verifier(); // Random state for CSRF protection

    // Build auth URL
    let auth_url = build_auth_url(&code_challenge, &state);

    // Open browser
    println!("Opening browser to authorize...");
    open::that(&auth_url).map_err(|e| anyhow!("Failed to open browser: {}", e))?;

    // Wait for callback (this blocks)
    let code = wait_for_callback(&state)?;

    // Exchange code for token
    println!("Exchanging authorization code...");
    let token = exchange_code(&code, &code_verifier).await?;

    Ok(token)
}

// We need urlencoding
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
