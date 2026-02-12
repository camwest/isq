//! Shared GraphQL query strings for Linear integration.
//!
//! Keeping query definitions in this module helps keep implementation files
//! focused and under the repo's size limits.

pub(super) const TEAM_LABELS_QUERY: &str = r#"
    query($teamId: String!) {
        team(id: $teamId) {
            labels {
                nodes {
                    name
                    color
                }
            }
        }
    }
"#;

pub(super) const TEAM_LABELS_WITH_IDS_QUERY: &str = r#"
    query($teamId: String!) {
        team(id: $teamId) {
            labels {
                nodes {
                    id
                    name
                    color
                }
            }
        }
    }
"#;

#[cfg(test)]
mod tests {
    use super::{TEAM_LABELS_QUERY, TEAM_LABELS_WITH_IDS_QUERY};

    #[test]
    fn team_labels_query_uses_string_team_id() {
        assert!(TEAM_LABELS_QUERY.contains("query($teamId: String!)"));
    }

    #[test]
    fn team_labels_with_ids_query_uses_string_team_id() {
        assert!(TEAM_LABELS_WITH_IDS_QUERY.contains("query($teamId: String!)"));
    }
}
