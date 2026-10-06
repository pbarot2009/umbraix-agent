use serde_json::{json, Value};

use super::{channels, moderation, roles};

/// Tools declaration using Gemini's native function calling schema.
///
/// Kept as one `function_declarations` block so the request shape is
/// identical to the original single-file bot.
pub fn build_tools_declaration() -> Value {
    json!([{
        "function_declarations": [
            roles::schema(),
            channels::schema(),
            moderation::schema(),
        ]
    }])
}

/// Names of all registered tools (used by tests and diagnostics).
pub fn tool_names() -> &'static [&'static str] {
    &["create_role", "rename_channel", "purge_messages"]
}

/// Whether a tool performs a destructive Discord action.
///
/// Used by the ReAct loop for extra logging; Phase 3 wires this into a
/// human confirmation gate.
pub fn is_destructive(name: &str) -> bool {
    matches!(name, "purge_messages")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_contains_all_tools() {
        let decl = build_tools_declaration();
        let fns = decl[0]["function_declarations"].as_array().unwrap();
        let names: Vec<&str> = fns.iter().filter_map(|f| f["name"].as_str()).collect();
        for expected in tool_names() {
            assert!(names.contains(expected), "missing tool {expected}");
        }
    }
}
