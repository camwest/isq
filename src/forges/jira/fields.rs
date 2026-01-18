//! JIRA field discovery operations

use anyhow::Result;
use serde::Deserialize;

use super::client::JiraClient;
use super::truncate;

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

impl JiraClient {
    /// List available JIRA fields (for JQL queries)
    pub async fn list_fields(&self) -> Result<()> {
        let fields: Vec<JiraField> = self.get("/field").await?;

        // Filter to searchable fields and sort by name
        let mut searchable: Vec<_> = fields.iter().filter(|f| f.searchable).collect();
        searchable.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        println!("{:<30} {:<25} {:<15} ID", "Name", "JQL Clause", "Type");
        println!("{}", "-".repeat(85));

        for field in &searchable {
            let clause = field
                .clause_names
                .first()
                .map(|s| s.as_str())
                .unwrap_or(&field.id);
            let field_type = field
                .schema
                .as_ref()
                .and_then(|s| s.field_type.as_deref())
                .unwrap_or("unknown");
            println!(
                "{:<30} {:<25} {:<15} {}",
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
}
