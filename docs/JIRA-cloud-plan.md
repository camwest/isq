# JIRA Cloud Forge Implementation Plan

Implementation plan for adding JIRA Cloud support to isq.

---

## Scope

**In Scope:**
- JIRA Cloud only (not Server/Data Center)
- Issues: list, show, create, close, reopen, comment, assign, labels
- Goals: configurable mapping (Versions or Epics)
- OAuth 2.0 (3LO) authentication with PKCE
- API token fallback for headless/CI
- Non-interactive project selection via `-o project=X` argument
- Workflow state transitions via `on_start`
- Rate limit handling
- JQL passthrough for advanced filtering

**Out of Scope (v1):**
- JIRA Server/Data Center (different API, auth, hosting)
- Sprints/Boards (Agile-specific, not universal)
- Components (JIRA-specific concept)
- Custom fields (too variable across instances)
- Sub-tasks (issue type complexity)
- Attachments
- Watchers
- Webhooks/real-time updates (polling only for v1)
- Read-only project support (fail fast if no write access)

---

## Decisions

### 1. Authentication

**Decision:** OAuth 2.0 (3LO) with PKCE as primary, API token fallback for CI.

**Rationale:**
- We already implemented OAuth for GitHub (device flow) and Linear (PKCE)
- OAuth provides better UX: browser opens, user consents, done
- Pushing API token creation onto users is worse UX
- JIRA Cloud OAuth 2.0 (3LO) uses standard authorization code flow with PKCE

**Token Lifetimes:**
- Access tokens: **1 hour** (not 15 minutes as previously documented)
- Refresh tokens: **90 days** (rotating - new token issued on each use)

**Token Refresh Strategy:**
- Check token expiry before each API call
- Refresh if <5 minutes remaining (following Linear pattern)
- On 401, attempt refresh once, then error with re-auth prompt

**OAuth Flow:**
1. Generate PKCE code verifier/challenge
2. Open browser to `https://auth.atlassian.com/authorize`
3. User consents to scopes on Atlassian site
4. Callback to local server with auth code
5. Exchange code for access + refresh tokens
6. Store in OS keyring

**Required Scopes:**
- `read:jira-work` - Read issues, projects
- `write:jira-work` - Create/update issues, comments
- `read:me` - Get current user
- `offline_access` - Refresh tokens

**OAuth App:** Shared public app - isq has its own registered app with public client_id. Users just authorize. App registration handled separately by maintainer.

**Callback Port:** Fixed at 19285, error if busy (same pattern as Linear).

**Consent Revocation:** When OAuth consent is revoked on Atlassian's side, show explicit error explaining tokens were revoked, require `isq link jira` to re-authenticate.

**Fallback (CI/headless):**
```bash
$ isq link jira --token
Paste your API token (create at https://id.atlassian.com/manage-profile/security/api-tokens):
Enter your email: user@acme.com
Enter site (e.g., acme.atlassian.net): acme.atlassian.net
```

**Env var format:** JSON for unambiguous parsing:
```bash
JIRA_API_TOKEN='{"email":"user@acme.com","token":"abc123","site":"acme.atlassian.net"}'
```

**Auth Config:**
```rust
pub const AUTH: AuthConfig = AuthConfig {
    keyring_service: "jira",
    env_var: "JIRA_API_TOKEN",
    cli_command: None,
    display_name: "Jira",
    link_command: "isq link jira",
};
```

### 2. API Version

**Decision:** REST API v3 (Cloud-only, modern).

**Rationale:**
- v3 is the current Cloud API
- Consistent response formats
- Active development and documentation

**Base URL:** `https://{site}.atlassian.net/rest/api/3/`

### 3. Project Selection

**Decision:** Refactor `LinkArgs` to be a generic options map. JIRA uses `project` / `list-projects`.

**Breaking Change:** Remove `--team`/`--list-teams` for Linear, require `-o team=X`. Clean break, no deprecation period.

**Refactor:**
```rust
// Before (Linear-specific fields in shared struct)
pub struct LinkArgs {
    pub team: Option<String>,
    pub list_teams: bool,
}

// After (generic options map, each forge interprets its own keys)
pub struct LinkArgs {
    pub options: HashMap<String, String>,
    pub flags: HashSet<String>,
}

impl LinkArgs {
    pub fn get(&self, key: &str) -> Option<&str> { ... }
    pub fn has_flag(&self, flag: &str) -> bool { ... }
}
```

**JIRA uses:**
- `-o project=PROJ` - select project
- `-o list-projects` - list available projects

**Linear uses:**
- `-o team=ENG` - select team
- `-o list-teams` - list available teams

**Implementation:**
```bash
# Auto-select if only one project
$ isq link jira
Using project: PROJ (Project Alpha)
✓ Synced 234 issues

# Explicit selection
$ isq link jira -o project=PROJ
✓ Synced 234 issues

# List available projects
$ isq link jira -o list-projects
Available projects:
  PROJ - Project Alpha
  TEAM - Team Beta

# Error if multiple projects and none specified
$ isq link jira
Error: Multiple projects available. Specify one with -o project=<key>.

Available projects:
  PROJ - Project Alpha
  TEAM - Team Beta

Example: isq link jira -o project="PROJ"
```

**Repo Identifier Format:** `{site}/{project_key}` (e.g., `acme.atlassian.net/PROJ`)

**Permission Check During Link:** Probe write capability during link. If user has read-only access (can view but not create/edit issues), error out. No partial read-only support in v1.

### 4. Issue State Mapping

**Decision:** Map JIRA status categories to open/closed.

JIRA has configurable workflows with many statuses, but each status belongs to a category:

| JIRA Status Category Key | isq State |
|-------------------------|-----------|
| `new`                   | open      |
| `indeterminate`         | open      |
| `done`                  | closed    |

**Implementation:**
```rust
fn map_state(status_category_key: &str) -> &'static str {
    match status_category_key {
        "done" => "closed",
        _ => "open",  // "new", "indeterminate"
    }
}
```

### 5. Goal Mapping

**Decision:** Configurable via `goal_type` setting. Default to Versions.

**Research findings:**
- **Versions** (Fix Versions): Used for release tracking, have target dates, simpler API
- **Epics**: Used for goal/OKR tracking, parent-child hierarchy, commonly used for quarterly goals
- Teams use both depending on methodology
- Atlas (Atlassian's goal tool) links to Epics

**Configuration in `.config/isq.toml`:**
```toml
[jira]
goal_type = "version"  # or "epic"
```

**Default:** `version` (simpler, universally available, has target dates)

**Progress Calculation:** Calculate locally - query issues with goal, count done vs total. Works offline, accurate.

**Version → Goal Mapping:**
| isq Goal field | JIRA Version field |
|----------------|-------------------|
| id             | id                |
| name           | name              |
| description    | description       |
| target_date    | releaseDate       |
| state          | released/archived → closed, else open |
| progress       | Calculate from issues with fixVersion |

**Epic → Goal Mapping:**
| isq Goal field | JIRA Epic field |
|----------------|--------------------|
| id             | id              |
| name           | summary         |
| description    | description (ADF) |
| target_date    | duedate (if set) |
| state          | status category done → closed |
| progress       | Calculate from child issues |

### 6. Issue Key & Number Format

**Decision:** Use full JIRA key (PROJ-123) as the display identifier.

**Rationale:**
- JIRA devs say "PROJ-123" in conversation, never just "123"
- Matches JIRA mental model
- Consider similar change for Linear (`WRK-207` instead of `#207`)

**Display:** `PROJ-123 Fix login bug`
**Internal ID:** JIRA's stable numeric `id` field (not the key number)

**Key Changes (Issue Moved Between Projects):**
When an issue is moved (e.g., `PROJ-123` → `OTHER-456`), update the display key silently. The internal JIRA ID stays stable. This is rare and matches what happens when GitHub issues transfer between repos.

**Show Command Syntax:** Support both:
- `isq show 123` - infers project from linked directory, resolves to PROJ-123
- `isq show OTHER-456` - explicit cross-project reference works

**URL Format:** `https://{site}.atlassian.net/browse/{key}`

### 7. Rate Limiting

**Decision:** Implement `get_rate_limit()` like GitHub and Linear. Parse headers, return `RateLimitInfo`.

**Research findings ([source](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/)):**
- JIRA Cloud uses a points-based model internally
- Burst limits (seconds) and hourly quotas enforced independently
- Returns headers on responses:
  - `X-RateLimit-Limit` - total allowed
  - `X-RateLimit-Remaining` - remaining
  - `X-RateLimit-Reset` - reset timestamp
  - `Retry-After` - seconds to wait (on 429)

**Implementation:**
```rust
async fn get_rate_limit(&self) -> Result<Option<RateLimitInfo>> {
    // Parse from last response headers (like Linear does)
    // Or make lightweight request and parse headers
    Ok(Some(RateLimitInfo {
        limit: parse_header("X-RateLimit-Limit"),
        remaining: parse_header("X-RateLimit-Remaining"),
        reset_at: parse_header("X-RateLimit-Reset"),
    }))
}
```

- Daemon uses `RateLimitInfo` for budget tracking (same as GitHub/Linear)
- On 429: parse `Retry-After`, exponential backoff (1s, 2s, 4s, 8s)
- Store in `rate_limit_state` table via `db::update_rate_limit_budget()`

### 8. Workflow Transitions (on_start)

**Decision:** Support transition by name, fetch available transitions dynamically.

JIRA workflows vary per project. The API (`GET /issue/{key}/transitions`) returns only transitions valid from the current status, so ambiguity is naturally resolved.

**on_start Config:**
```toml
transition = "In Progress"  # or "Start Progress", workflow-dependent
assign_self = true
```

**Implementation:**
1. GET `/rest/api/3/issue/{key}/transitions` → list available transitions from current status
2. Find transition matching name (case-insensitive)
3. POST `/rest/api/3/issue/{key}/transitions` with transition ID

**Validation:** `validate_on_start_config` checks TOML structure, not transition validity (runtime check).

### 9. Labels

**Decision:** Use JIRA's built-in labels field.

JIRA has a native `labels` field (array of strings, no colors).

**Implementation:**
- GET labels from issue response
- PUT to update labels (replace entire array)
- No auto-create needed (labels are freeform strings)

### 10. Issue Types

**Decision:** Expose issue types as pseudo-labels (e.g., `type:Bug`).

**Rationale:**
- JIRA devs filter by type constantly
- Pseudo-label is visible, filterable, requires no schema changes
- Support `isq list --type=Bug` as sugar for filtering

**Create Default:**
- During `isq link jira`, fetch project's default issue type from its issue type scheme
- Store in `.config/isq.toml` as `default_issue_type = "Task"`
- `isq create "Fix bug"` uses stored default
- `isq create "Fix bug" --type=Bug` overrides
- The `--type` flag is forge-opaque (not in shared abstraction, passes through to JIRA impl)

### 11. Priorities

**Decision:** Map JIRA's 5-level priority to isq's 3-level.

| JIRA Priority | isq Priority |
|---------------|--------------|
| Highest       | high         |
| High          | high         |
| Medium        | medium       |
| Low           | low          |
| Lowest        | low          |

### 12. Assignee

**Unassign:** `PUT` with `assignee: null` - anyone can unassign any issue, matches web UI behavior.

### 13. Comments

**Decision:** Sync comment edits.

- Track comment IDs in local DB
- Update body when comment is edited in JIRA
- More accurate sync, maintains parity with remote

### 14. JQL Passthrough

**Decision:** Support `--jql` flag for advanced filtering, plus `isq jira list-fields` command.

**Rationale:**
- Claude Code users can craft JQL queries effectively
- Power users get full JIRA query power
- Flag stays forge-opaque (not in shared abstraction)

**Commands:**
```bash
# Advanced filtering with JQL
isq list --jql="assignee = currentUser() AND priority = High"

# Discover available fields for JQL
isq jira list-fields
```

This requires forge-specific command routing pattern:
```bash
isq <forge> <subcommand>  # e.g., isq jira list-fields
```

### 15. Description Format (ADF)

**Reading (ADF → Markdown):**
- Parse ADF JSON, walk nodes, convert to Markdown
- Handle: paragraph, heading, text, link, code, list, mention
- **Media handling:** Placeholder text - `[Image: filename.png]`, `[@john.doe]`
- Fallback: extract plain text if complex/unknown nodes

**Writing (Markdown → ADF):**
- Use two existing Rust crates:
  1. `comrak` or `pulldown-cmark` - Markdown → HTML
  2. `htmltoadf` - HTML → ADF
- Well-tested approach with maintained libraries

### 16. Timestamps

**Decision:** Display in local timezone.

Convert all ISO 8601 timestamps from JIRA to user's local timezone for display. This is what developers expect - dates should feel natural.

### 17. Sync Behavior

**Cadence:** Same as other forges (5 minute default daemon interval).

**Deleted Issues:** Soft delete - mark as deleted in local DB, hide from list. Preserves history, worktree links stay intact.

**Access Removed (403):** When we get 403 Forbidden on a project (user access removed), error with clear message about access removal. Suggest running `isq link jira` to re-authenticate.

**Worktree Orphaning:** If user re-links directory to different project, old worktrees are orphaned silently. User cleans up manually if needed. Rare case, LLM assistant can help.

---

## API Mapping

### Issues

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| List all  | `GET /search?jql=project={key}&maxResults=100&startAt={offset}` |
| Get one   | `GET /issue/{key}` |
| Create    | `POST /issue` |
| Update    | `PUT /issue/{key}` |
| Close     | `POST /issue/{key}/transitions` (to Done status) |
| Reopen    | `POST /issue/{key}/transitions` (to To Do status) |

### Comments

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| List all  | `GET /search?jql=project={key}&expand=renderedFields,comment` |
| Add       | `POST /issue/{key}/comment` |

### Versions (Goals when goal_type=version)

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| List      | `GET /project/{key}/versions` |
| Create    | `POST /version` |
| Update    | `PUT /version/{id}` |
| Close     | `PUT /version/{id}` with `released: true` |

### Epics (Goals when goal_type=epic)

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| List      | `GET /search?jql=project={key} AND issuetype=Epic` |
| Create    | `POST /issue` with `issuetype: Epic` |
| Close     | `POST /issue/{key}/transitions` (to Done) |

### Labels

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| Add       | `PUT /issue/{key}` with updated labels array |
| Remove    | `PUT /issue/{key}` with updated labels array |

### User

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| Current   | `GET /myself` |
| Assign    | `PUT /issue/{key}/assignee` |

### Custom Fields

| Operation | JIRA API Endpoint |
|-----------|-------------------|
| List      | `GET /field` |

---

## OAuth 2.0 (3LO) Implementation

Based on [Atlassian OAuth 2.0 (3LO) docs](https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/):

### App Registration

Register app in [Atlassian Developer Console](https://developer.atlassian.com/console/myapps/):
- App type: OAuth 2.0 (3LO)
- Callback URL: `http://127.0.0.1:19285/callback`
- Scopes: `read:jira-work`, `write:jira-work`, `read:me`, `offline_access`

### Authorization Flow

```
┌─────────────┐     ┌─────────────────┐     ┌──────────────────┐
│   isq CLI   │     │  Local Server   │     │   Atlassian      │
└──────┬──────┘     └────────┬────────┘     └────────┬─────────┘
       │                     │                       │
       │ 1. Generate PKCE    │                       │
       │    code_verifier    │                       │
       │                     │                       │
       │ 2. Start server on  │                       │
       │    port 19285       │                       │
       │                     │                       │
       │ 3. Open browser ────┼───────────────────────►
       │    with auth URL    │                       │
       │                     │                       │
       │                     │     4. User consents  │
       │                     │                       │
       │                     │ ◄─────────────────────┤
       │                     │  5. Redirect with     │
       │                     │     auth code         │
       │                     │                       │
       │ ◄───────────────────┤                       │
       │  6. Return code     │                       │
       │                     │                       │
       │ 7. Exchange code ───┼───────────────────────►
       │    for tokens       │                       │
       │                     │                       │
       │ ◄───────────────────┼───────────────────────┤
       │  8. Access + refresh tokens                 │
       │                     │                       │
       │ 9. Store in keyring │                       │
```

### Token Refresh

Access tokens expire in 1 hour. Implement refresh:
```rust
async fn refresh_if_needed(&self) -> Result<()> {
    // Check if expires_at < now + 5 minutes
    // If so, POST to https://auth.atlassian.com/oauth/token
    // grant_type=refresh_token
    // refresh_token=...
    // client_id=...
    // Update stored tokens
}
```

### Site Selection

After OAuth, user may have access to multiple Atlassian sites. Need to:
1. GET `https://api.atlassian.com/oauth/token/accessible-resources`
2. Returns list of sites user can access
3. Auto-select if one site, prompt for `--site` if multiple

---

## Data Mapping

### Issue Response → isq Issue

```rust
Issue {
    number: jira.key.clone(),  // "PROJ-123" - full key as identifier
    title: jira.fields.summary.clone(),
    body: jira.fields.description.map(|d| adf_to_markdown(d)),
    state: map_state(&jira.fields.status.statusCategory.key),
    author: jira.fields.reporter.map(|r| r.displayName).unwrap_or_default(),
    labels: {
        let mut labels: Vec<Label> = jira.fields.labels.iter()
            .map(|l| Label { name: l.clone(), color: None })
            .collect();
        // Add issue type as pseudo-label
        labels.push(Label {
            name: format!("type:{}", jira.fields.issuetype.name),
            color: None
        });
        labels
    },
    priority: map_priority(&jira.fields.priority.name),
    created_at: jira.fields.created,
    updated_at: jira.fields.updated,
    url: Some(format!("https://{}/browse/{}", site, jira.key)),
    milestone: jira.fields.fixVersions.first().map(|v| v.name.clone()),
}
```

---

## File Structure

```
src/forges/
├── mod.rs          # Add JiraClient to ForgeType enum, refactor LinkArgs
├── github.rs       # Existing
├── linear.rs       # Existing (update for LinkArgs refactor)
└── jira.rs         # New: ~1000-1200 lines estimated

src/cli/
├── link.rs         # Add jira subcommand, refactor for -o options
└── jira.rs         # New: forge-specific commands (list-fields)
```

### jira.rs Structure

```rust
// Auth
pub const AUTH: AuthConfig = ...;
pub const DEFAULT_ON_START_TOML: &str = ...;

// OAuth constants
const JIRA_CLIENT_ID: &str = "...";  // From Atlassian Developer Console
const JIRA_AUTH_URL: &str = "https://auth.atlassian.com/authorize";
const JIRA_TOKEN_URL: &str = "https://auth.atlassian.com/oauth/token";
const REDIRECT_PORT: u16 = 19285;

// Types
struct JiraCredentials {
    access_token: String,
    refresh_token: Option<String>,
    site: String,
    expires_at: Option<String>,
}
struct JiraClient {
    client: reqwest::Client,
    creds: RwLock<JiraCredentials>,
}

// OAuth flow
pub async fn oauth_flow() -> Result<TokenResponse>

// Link flow
pub async fn link(repo_path: &str, args: &LinkArgs) -> Result<LinkResult>

// Forge trait impl
impl Forge for JiraClient { ... }

// Helpers
impl JiraClient {
    fn new(creds: JiraCredentials) -> Self
    async fn request<T>(&self, method: Method, path: &str) -> Result<T>
    async fn refresh_if_needed(&self) -> Result<()>
    async fn paginate<T>(&self, path: &str) -> Result<Vec<T>>
}

// ADF conversion
fn adf_to_markdown(adf: &Value) -> String
fn markdown_to_adf(md: &str) -> Value  // via comrak + htmltoadf
```

---

## Implementation Order

1. **LinkArgs refactor** - Generic options map, update Linear to use it
2. **OAuth flow** - PKCE, local callback server, token exchange
3. **Token storage** - Keyring with refresh token support, expiry tracking
4. **JiraClient basics** - HTTP client, auth headers, token refresh
5. **Site/project resolution** - accessible-resources API, project list, permission check
6. **List issues** - Search API, pagination, Issue mapping, type pseudo-labels
7. **Link flow** - Non-interactive project selection, initial sync, default issue type storage
8. **Show issue** - Single issue fetch, ADF→Markdown
9. **Comments** - List, create, sync edits
10. **State changes** - Close/reopen via transitions
11. **Labels** - Add/remove
12. **Assign** - Assignee update, unassign
13. **Create issue** - POST with required fields, --type flag
14. **Goals (Versions)** - List, create, progress calculation
15. **Goals (Epics)** - Alternative goal type
16. **on_start** - Transition + assign workflow
17. **Rate limiting** - Parse headers, backoff
18. **JQL passthrough** - --jql flag, isq jira list-fields command
19. **Tests** - Unit tests, integration tests

---

## Testing Strategy

### Unit Tests
- ADF to Markdown conversion
- Markdown to ADF conversion (via HTML)
- Issue/Goal mapping functions
- State category mapping
- Priority mapping
- Issue key parsing
- PKCE code generation
- JSON token parsing

### Integration Tests (with mock server)
- OAuth flow
- Token refresh
- Pagination handling
- Error responses (401, 403, 404, 429)
- Transition workflow
- Comment sync with edits

### Manual Testing
- Real JIRA Cloud instance (free tier)
- Default project configuration
- Various workflow schemes
- Test permission edge cases

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| ADF complexity | Start with common nodes, log unknown, use placeholders for media |
| Workflow variation | Fetch transitions dynamically (API returns only valid ones), clear error on missing |
| Multi-site access | Require `--site` arg if multiple, auto-select if one |
| Token refresh timing | Check before each request, refresh if <5min remaining, retry on 401 |
| Epic hierarchy complexity | Start with Versions default, Epic as opt-in |
| htmltoadf limitations | Test thoroughly, fallback to plain text if conversion fails |

---

## Success Criteria

- [ ] `isq link jira` authenticates via OAuth and syncs issues
- [ ] `isq link jira -o project=X` selects specific project (non-interactive)
- [ ] Permission check during link fails fast on read-only access
- [ ] `isq list` shows JIRA issues with full keys (PROJ-123) and type labels
- [ ] `isq show PROJ-123` displays issue with formatted body
- [ ] `isq show 123` infers project from linked directory
- [ ] `isq create` creates issue in JIRA with default type
- [ ] `isq create --type=Bug` overrides issue type
- [ ] `isq close/reopen` transitions issue state
- [ ] `isq comment` adds comment
- [ ] `isq assign` updates assignee
- [ ] `isq assign --unassign` clears assignee
- [ ] `isq label add/remove` modifies labels
- [ ] `isq goal list` shows versions (or epics if configured)
- [ ] `isq start` triggers on_start workflow
- [ ] `isq list --jql="..."` filters with raw JQL
- [ ] `isq jira list-fields` shows available fields
- [ ] Rate limits handled gracefully (429 → backoff)
- [ ] Token refresh works transparently (1-hour access, 90-day refresh)
- [ ] Works offline with cached data
- [ ] Soft delete preserves history for deleted issues
- [ ] Comments sync including edits

---

## References

- [JIRA Cloud REST API v3](https://developer.atlassian.com/cloud/jira/platform/rest/v3/intro/)
- [OAuth 2.0 (3LO) Apps](https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/)
- [Scopes for OAuth 2.0](https://developer.atlassian.com/cloud/jira/platform/scopes-for-oauth-2-3LO-and-forge-apps/)
- [Rate Limiting](https://developer.atlassian.com/cloud/jira/platform/rate-limiting/)
- [JQL Reference](https://support.atlassian.com/jira-service-management-cloud/docs/use-advanced-search-with-jira-query-language-jql/)
- [ADF Schema](https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/)
- [htmltoadf crate](https://crates.io/crates/htmltoadf)
