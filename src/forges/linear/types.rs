//! GraphQL response types for Linear API

use serde::{Deserialize, Serialize};

use crate::forges::{Goal, GoalState};

// ============================================================================
// Core GraphQL Types
// ============================================================================

#[derive(Deserialize)]
pub struct GraphQLResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQLError>>,
}

#[derive(Deserialize)]
pub struct GraphQLError {
    pub message: String,
}

#[derive(Serialize)]
pub struct GraphQLRequest {
    pub query: String,
    pub variables: Option<serde_json::Value>,
}

// ============================================================================
// User & Organization
// ============================================================================

#[derive(Deserialize)]
pub struct ViewerResponse {
    pub viewer: LinearUser,
}

#[derive(Deserialize)]
pub struct LinearUser {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct OrganizationResponse {
    pub organization: LinearOrganization,
}

#[derive(Deserialize, Clone)]
pub struct LinearOrganization {
    #[serde(rename = "urlKey")]
    pub url_key: String,
}

#[derive(Deserialize)]
pub struct UsersResponse {
    pub users: UserConnection,
}

#[derive(Deserialize)]
pub struct UserConnection {
    pub nodes: Vec<LinearUserWithId>,
}

#[derive(Deserialize)]
pub struct LinearUserWithId {
    pub id: String,
    pub name: String,
    pub email: String,
}

// ============================================================================
// Teams
// ============================================================================

#[derive(Deserialize)]
pub struct TeamsResponse {
    pub teams: TeamConnection,
}

#[derive(Deserialize)]
pub struct TeamConnection {
    pub nodes: Vec<LinearTeam>,
}

#[derive(Deserialize, Clone)]
pub struct LinearTeam {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Deserialize)]
pub struct TeamLabelsResponse {
    pub team: TeamWithLabels,
}

#[derive(Deserialize)]
pub struct TeamWithLabels {
    pub labels: TeamLabelConnection,
}

#[derive(Deserialize)]
pub struct TeamLabelConnection {
    pub nodes: Vec<LinearLabelWithId>,
}

// ============================================================================
// Issues
// ============================================================================

#[derive(Deserialize)]
pub struct IssuesResponse {
    pub issues: IssueConnection,
}

#[derive(Deserialize)]
pub struct IssueConnection {
    pub nodes: Vec<LinearIssue>,
    #[serde(rename = "pageInfo")]
    pub page_info: Option<PageInfo>,
}

#[derive(Deserialize, Clone)]
pub struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

#[derive(Deserialize)]
pub struct LinearIssue {
    pub identifier: String,
    #[allow(dead_code)]
    pub number: u64, // From API but unused - we use identifier instead
    pub title: String,
    pub description: Option<String>,
    pub state: LinearState,
    pub creator: Option<LinearCreator>,
    pub assignee: Option<LinearAssignee>,
    /// Linear priority: 0=no priority, 1=urgent, 2=high, 3=normal, 4=low
    pub priority: u8,
    pub labels: LabelConnection,
    pub project: Option<LinearProjectRef>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct LinearState {
    #[serde(rename = "type")]
    pub state_type: String,
}

#[derive(Deserialize)]
pub struct LinearCreator {
    pub name: String,
}

#[derive(Deserialize)]
pub struct LinearAssignee {
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct LabelConnection {
    pub nodes: Vec<LinearLabel>,
}

#[derive(Deserialize)]
pub struct LinearLabel {
    pub name: String,
    pub color: String,
}

/// Minimal project info embedded in issue responses
#[derive(Deserialize)]
pub struct LinearProjectRef {
    pub name: String,
}

#[derive(Deserialize)]
pub struct SingleIssueListResponse {
    pub issues: IssueConnectionWithDetails,
}

#[derive(Deserialize)]
pub struct IssueConnectionWithDetails {
    pub nodes: Vec<LinearIssueWithDetails>,
}

#[derive(Deserialize)]
pub struct LinearIssueWithDetails {
    pub id: String,
    pub labels: LabelConnectionWithIds,
}

#[derive(Deserialize)]
pub struct LabelConnectionWithIds {
    pub nodes: Vec<LinearLabelWithId>,
}

#[derive(Deserialize)]
pub struct LinearLabelWithId {
    pub id: String,
    pub name: String,
}

// ============================================================================
// Workflow States
// ============================================================================

#[derive(Deserialize)]
pub struct WorkflowStatesResponse {
    #[serde(rename = "workflowStates")]
    pub workflow_states: WorkflowStateConnection,
}

#[derive(Deserialize)]
pub struct WorkflowStateConnection {
    pub nodes: Vec<WorkflowState>,
}

#[derive(Deserialize, Clone)]
pub struct WorkflowState {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub state_type: String,
}

// ============================================================================
// Comments
// ============================================================================

#[derive(Deserialize)]
pub struct CommentsResponse {
    pub comments: CommentsConnection,
}

#[derive(Deserialize)]
pub struct CommentsConnection {
    pub nodes: Vec<LinearCommentWithIssue>,
    #[serde(rename = "pageInfo")]
    pub page_info: Option<PageInfo>,
}

#[derive(Deserialize)]
pub struct LinearCommentWithIssue {
    pub id: String,
    pub body: String,
    pub user: Option<LinearCommentUser>,
    pub issue: CommentIssueRef,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct CommentIssueRef {
    pub identifier: String,
}

#[derive(Deserialize)]
pub struct LinearCommentUser {
    pub name: String,
}

// ============================================================================
// Mutation Responses
// ============================================================================

#[derive(Deserialize)]
pub struct IssueCreateResponse {
    #[serde(rename = "issueCreate")]
    pub issue_create: IssueCreatePayload,
}

#[derive(Deserialize)]
pub struct IssueCreatePayload {
    pub issue: CreatedIssue,
}

#[derive(Deserialize)]
pub struct CreatedIssue {
    pub identifier: String,
    #[allow(dead_code)]
    pub number: u64, // From API but unused - we use identifier instead
    pub title: String,
}

#[derive(Deserialize)]
pub struct CommentCreateResponse {
    #[serde(rename = "commentCreate")]
    pub comment_create: CommentCreatePayload,
}

#[derive(Deserialize)]
pub struct CommentCreatePayload {
    pub success: bool,
}

#[derive(Deserialize)]
pub struct IssueUpdateResponse {
    #[serde(rename = "issueUpdate")]
    pub issue_update: IssueUpdatePayload,
}

#[derive(Deserialize)]
pub struct IssueUpdatePayload {
    pub success: bool,
}

// ============================================================================
// Projects (Goals)
// ============================================================================

#[derive(Deserialize)]
pub struct ProjectsResponse {
    pub projects: ProjectConnection,
}

#[derive(Deserialize)]
pub struct ProjectConnection {
    pub nodes: Vec<LinearProject>,
}

#[derive(Deserialize, Clone)]
pub struct LinearProject {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub state: String,
    #[serde(rename = "targetDate")]
    pub target_date: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub url: String,
    pub progress: f64,
}

impl From<LinearProject> for Goal {
    fn from(p: LinearProject) -> Self {
        Goal {
            id: p.id,
            name: p.name,
            description: p.description,
            target_date: p.target_date,
            state: match p.state.as_str() {
                "completed" | "canceled" => GoalState::Closed,
                _ => GoalState::Open,
            },
            progress: p.progress,
            open_count: None,  // Linear doesn't provide counts efficiently
            closed_count: None,
            created_at: p.created_at,
            updated_at: p.updated_at,
            html_url: Some(p.url),
        }
    }
}

#[derive(Deserialize)]
pub struct ProjectCreateResponse {
    #[serde(rename = "projectCreate")]
    pub project_create: ProjectCreatePayload,
}

#[derive(Deserialize)]
pub struct ProjectCreatePayload {
    pub success: bool,
    pub project: Option<LinearProject>,
}

#[derive(Deserialize)]
pub struct ProjectUpdateResponse {
    #[serde(rename = "projectUpdate")]
    pub project_update: ProjectUpdatePayload,
}

#[derive(Deserialize)]
pub struct ProjectUpdatePayload {
    pub success: bool,
}
