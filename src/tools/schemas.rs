use serde_json::{json, Value};

use super::{channels, info, moderation, roles};

/// Tools declaration using Gemini's native function calling schema.
///
/// Kept as one `function_declarations` block so the request shape matches
/// what the model expects. Every tool the AI may call must be listed here
/// AND in the dispatcher in `mod.rs` — tests enforce the two stay in sync.
pub fn build_tools_declaration() -> Value {
    json!([{
        "function_declarations": [
            // Read-only grounding (list first so the model discovers them).
            info::list_channels_schema(),
            info::user_info_schema(),
            // Moderation.
            moderation::kick_schema(),
            moderation::ban_schema(),
            moderation::unban_schema(),
            moderation::timeout_schema(),
            moderation::purge_schema(),
            // Channels.
            channels::create_schema(),
            channels::rename_schema(),
            channels::delete_schema(),
            channels::slowmode_schema(),
            // Roles.
            roles::create_schema(),
            roles::assign_schema(),
            roles::remove_schema(),
        ]
    }])
}

/// Names of all registered tools (used by tests and diagnostics).
pub fn tool_names() -> &'static [&'static str] {
    &[
        "list_channels",
        "user_info",
        "kick_member",
        "ban_member",
        "unban_user",
        "timeout_member",
        "purge_messages",
        "create_channel",
        "rename_channel",
        "delete_channel",
        "set_slowmode",
        "create_role",
        "assign_role",
        "remove_role",
    ]
}

/// Whether a tool performs a destructive or hard-to-reverse Discord action.
///
/// Used by the ReAct loop for extra logging; a human confirmation gate
/// can be wired to this in a later phase.
pub fn is_destructive(name: &str) -> bool {
    matches!(
        name,
        "kick_member" | "ban_member" | "timeout_member" | "purge_messages" | "delete_channel"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn declaration_contains_all_tools() {
        let decl = build_tools_declaration();
        let fns = decl[0]["function_declarations"].as_array().unwrap();
        let names: HashSet<&str> = fns.iter().filter_map(|f| f["name"].as_str()).collect();
        let expected: HashSet<&str> = tool_names().iter().copied().collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn every_declaration_has_params_object() {
        let decl = build_tools_declaration();
        for f in decl[0]["function_declarations"].as_array().unwrap() {
            assert!(
                f["description"].is_string(),
                "tool missing description: {f}"
            );
            assert_eq!(f["parameters"]["type"], "object", "bad params: {f}");
        }
    }
}
