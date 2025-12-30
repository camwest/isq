# JIRA OAuth Proxy Service

A minimal OAuth proxy service for isq's JIRA Cloud integration.

## Why This Exists

JIRA Cloud OAuth 2.0 requires a `client_secret` for token exchange. Since isq is open-source, we cannot embed secrets in the CLI binary. This service holds the secret server-side and acts as an intermediary during OAuth flows.

## How It Works

```
CLI                         Service (Fly.io)              Atlassian
 │                              │                            │
 │ 1. Generate PKCE             │                            │
 │    (code_verifier)           │                            │
 │                              │                            │
 │ 2. Open browser ─────────────┼───────────────────────────►│
 │    (code_verifier in state)  │                            │
 │                              │                            │
 │                              │◀─── 3. Redirect with code ─│
 │                              │                            │
 │                              │ 4. Exchange code+secret ──►│
 │                              │◀─── tokens ────────────────│
 │                              │                            │
 │◀── 5. Redirect to localhost ─│                            │
 │    with tokens (base64)      │                            │
```

1. CLI generates PKCE code_verifier and encodes it in the OAuth `state` parameter
2. User authorizes in browser, Atlassian redirects to this service with auth code
3. Service extracts code_verifier from state, exchanges code + client_secret for tokens
4. Service redirects to `http://127.0.0.1:19285/callback?tokens=<base64>`
5. CLI's local server receives tokens directly (no secret exposure)

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Health check |
| `/callback` | GET | OAuth callback from Atlassian |
| `/refresh` | POST | Token refresh (JSON body: `{"refresh_token": "..."}`) |

## Environment Variables

```
JIRA_CLIENT_ID=...      # From Atlassian Developer Console
JIRA_CLIENT_SECRET=...  # From Atlassian Developer Console (secret!)
```

## Deployment

Deployed to Fly.io at `isq-jira-oauth.fly.dev`:

```bash
cd services/jira-oauth
fly deploy
```

## Local Development

```bash
cargo run
# Runs on http://localhost:3000
```

For local testing, set the callback URL in Atlassian Developer Console to `http://localhost:3000/callback`.
