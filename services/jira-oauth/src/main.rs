use axum::{
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::env;

const JIRA_TOKEN_URL: &str = "https://auth.atlassian.com/oauth/token";
const LOCAL_CALLBACK: &str = "http://127.0.0.1:19285/callback";
const REDIRECT_URI: &str = "https://isq-jira-oauth.fly.dev/callback";

#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct AtlassianTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Serialize)]
struct TokensPayload {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Serialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/", get(health))
        .route("/callback", get(callback))
        .route("/refresh", post(refresh));

    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("0.0.0.0:{}", port);

    println!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> &'static str {
    "ok"
}

async fn callback(Query(params): Query<CallbackParams>) -> impl IntoResponse {
    // Check for error from Atlassian
    if let Some(error) = params.error {
        let desc = params.error_description.unwrap_or_else(|| "Unknown error".to_string());
        let msg = format!("OAuth error: {} - {}", error, desc);
        let encoded = URL_SAFE_NO_PAD.encode(msg.as_bytes());
        return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
    }

    let code = match params.code {
        Some(c) => c,
        None => {
            let encoded = URL_SAFE_NO_PAD.encode(b"Missing code parameter");
            return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
        }
    };

    let state = match params.state {
        Some(s) => s,
        None => {
            let encoded = URL_SAFE_NO_PAD.encode(b"Missing state parameter");
            return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
        }
    };

    // Decode code_verifier from state
    let code_verifier = match URL_SAFE_NO_PAD.decode(&state) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(_) => {
                let encoded = URL_SAFE_NO_PAD.encode(b"Invalid state encoding");
                return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
            }
        },
        Err(_) => {
            let encoded = URL_SAFE_NO_PAD.encode(b"Failed to decode state");
            return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
        }
    };

    // Get secrets from environment
    let client_id = env::var("JIRA_CLIENT_ID")
        .unwrap_or_else(|_| "VG2jV3YlB3mSWdHcLRZJ8kawl6BFWki8".to_string());
    let client_secret = match env::var("JIRA_CLIENT_SECRET") {
        Ok(s) => s,
        Err(_) => {
            let encoded = URL_SAFE_NO_PAD.encode(b"JIRA_CLIENT_SECRET not configured");
            return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
        }
    };

    // Exchange code for tokens
    let client = reqwest::Client::new();
    let response = client
        .post(JIRA_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("redirect_uri", REDIRECT_URI),
            ("code", &code),
            ("code_verifier", &code_verifier),
        ])
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("Token request failed: {}", e);
            let encoded = URL_SAFE_NO_PAD.encode(msg.as_bytes());
            return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let msg = format!("Token exchange failed ({}): {}", status, body);
        let encoded = URL_SAFE_NO_PAD.encode(msg.as_bytes());
        return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
    }

    let tokens: AtlassianTokenResponse = match response.json().await {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("Failed to parse token response: {}", e);
            let encoded = URL_SAFE_NO_PAD.encode(msg.as_bytes());
            return Redirect::temporary(&format!("{}?error={}", LOCAL_CALLBACK, encoded));
        }
    };

    // Encode tokens for redirect
    let payload = TokensPayload {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_in: tokens.expires_in,
    };
    let json = serde_json::to_string(&payload).unwrap();
    let encoded = URL_SAFE_NO_PAD.encode(json.as_bytes());

    Redirect::temporary(&format!("{}?tokens={}", LOCAL_CALLBACK, encoded))
}

async fn refresh(Json(req): Json<RefreshRequest>) -> impl IntoResponse {
    let client_id = env::var("JIRA_CLIENT_ID")
        .unwrap_or_else(|_| "VG2jV3YlB3mSWdHcLRZJ8kawl6BFWki8".to_string());
    let client_secret = match env::var("JIRA_CLIENT_SECRET") {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "JIRA_CLIENT_SECRET not configured".to_string(),
                }),
            )
                .into_response();
        }
    };

    let client = reqwest::Client::new();
    let response = client
        .post(JIRA_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &client_id),
            ("client_secret", &client_secret),
            ("refresh_token", &req.refresh_token),
        ])
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: format!("Token request failed: {}", e),
                }),
            )
                .into_response();
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse {
                error: format!("Token refresh failed ({}): {}", status, body),
            }),
        )
            .into_response();
    }

    let tokens: AtlassianTokenResponse = match response.json().await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse {
                    error: format!("Failed to parse token response: {}", e),
                }),
            )
                .into_response();
        }
    };

    Json(RefreshResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_in: tokens.expires_in,
    })
    .into_response()
}
