//! Atlassian Document Format (ADF) conversion utilities

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
            let level = node
                .get("attrs")
                .and_then(|a| a.get("level"))
                .and_then(|l| l.as_u64())
                .unwrap_or(1);
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
                        "strong" => {
                            prefix.push_str("**");
                            suffix.insert_str(0, "**");
                        }
                        "em" => {
                            prefix.push('*');
                            suffix.insert(0, '*');
                        }
                        "code" => {
                            prefix.push('`');
                            suffix.insert(0, '`');
                        }
                        "strike" => {
                            prefix.push_str("~~");
                            suffix.insert_str(0, "~~");
                        }
                        "link" => {
                            if let Some(href) = mark
                                .get("attrs")
                                .and_then(|a| a.get("href"))
                                .and_then(|h| h.as_str())
                            {
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
            let language = node
                .get("attrs")
                .and_then(|a| a.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("");
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
            let text = node
                .get("attrs")
                .and_then(|a| a.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("@user");
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
            let media_type = node
                .get("attrs")
                .and_then(|a| a.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("file");
            let name = node
                .get("attrs")
                .and_then(|a| a.get("alt"))
                .and_then(|t| t.as_str())
                .or_else(|| {
                    node.get("attrs")
                        .and_then(|a| a.get("id"))
                        .and_then(|t| t.as_str())
                })
                .unwrap_or("attachment");
            output.push_str(&format!("[{}: {}]", media_type.to_uppercase(), name));
        }
        "emoji" => {
            let shortname = node
                .get("attrs")
                .and_then(|a| a.get("shortName"))
                .and_then(|t| t.as_str())
                .unwrap_or(":emoji:");
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
