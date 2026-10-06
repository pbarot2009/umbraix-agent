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
    for name in ["create_role", "rename_channel", "purge_messages"] {
        assert!(tools::tool_names().contains(&name));
    }
}

#[test]
fn destructive_flags_are_sane() {
    assert!(tools::is_destructive("purge_messages"));
    assert!(!tools::is_destructive("create_role"));
    assert!(!tools::is_destructive("rename_channel"));
}
