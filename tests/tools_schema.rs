use discord_gemini_agent::tools;

#[test]
fn tools_declaration_shape_matches_gemini_api() {
    let decl = tools::build_tools_declaration();
    let blocks = decl.as_array().expect("top-level array");
    assert_eq!(blocks.len(), 1);
    let fns = blocks[0]["function_declarations"]
        .as_array()
        .expect("function_declarations array");
    assert_eq!(fns.len(), tools::tool_names().len());

    for f in fns {
        assert!(f["name"].is_string(), "tool missing name: {f}");
        assert!(
            f["description"].is_string(),
            "tool missing description: {f}"
        );
        assert_eq!(f["parameters"]["type"], "object");
    }
}

#[test]
fn registry_lists_expected_tools() {
    for name in [
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
    ] {
        assert!(tools::tool_names().contains(&name), "missing {name}");
    }
    // No duplicates.
    let mut seen = std::collections::HashSet::new();
    for name in tools::tool_names() {
        assert!(seen.insert(name), "duplicate tool {name}");
    }
}

#[test]
fn destructive_flags_are_sane() {
    for name in [
        "kick_member",
        "ban_member",
        "timeout_member",
        "purge_messages",
        "delete_channel",
    ] {
        assert!(tools::is_destructive(name), "{name} should be destructive");
    }
    for name in [
        "list_channels",
        "user_info",
        "create_role",
        "rename_channel",
        "set_slowmode",
    ] {
        assert!(
            !tools::is_destructive(name),
            "{name} should not be destructive"
        );
    }
}
