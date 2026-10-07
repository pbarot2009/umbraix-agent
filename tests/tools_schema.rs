use umbraix_agent::tools;

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
        "channel_details",
        "list_active_threads",
        "user_info",
        "list_roles",
        "role_info",
        "list_role_members",
        "search_members",
        "list_members",
        "server_info",
        "list_bans",
        "get_messages",
        "get_message_details",
        "get_audit_logs",
        "list_emojis",
        "list_invites",
        "list_channel_invites",
        "list_pins",
        "list_webhooks",
        "list_scheduled_events",
        "list_automod_rules",
        "list_thread_members",
        "get_voice_state",
        "prune_preview",
        "send_message",
        "edit_message",
        "delete_single_message",
        "pin_message",
        "unpin_message",
        "publish_message",
        "kick_member",
        "ban_member",
        "unban_user",
        "timeout_member",
        "remove_timeout",
        "purge_messages",
        "set_nickname",
        "warn_member",
        "prune_members",
        "create_channel",
        "rename_channel",
        "delete_channel",
        "move_channel",
        "clone_channel",
        "set_slowmode",
        "set_topic",
        "set_channel_nsfw",
        "set_voice_limits",
        "lock_channel",
        "unlock_channel",
        "set_channel_permissions",
        "clear_channel_permissions",
        "create_role",
        "edit_role",
        "set_role_color",
        "set_role_permissions",
        "set_role_position",
        "assign_role",
        "remove_role",
        "delete_role",
        "move_voice_member",
        "disconnect_voice_member",
        "mute_voice_member",
        "unmute_voice_member",
        "deafen_voice_member",
        "undeafen_voice_member",
        "create_thread",
        "thread_from_message",
        "join_thread",
        "archive_thread",
        "add_thread_member",
        "remove_thread_member",
        "create_invite",
        "delete_invite",
        "create_webhook",
        "delete_webhook",
        "rename_emoji",
        "delete_emoji",
        "delete_scheduled_event",
        "edit_server",
    ] {
        assert!(tools::tool_names().contains(&name), "missing {name}");
    }
    assert!(
        tools::tool_names().len() >= 60,
        "need 60+ tools, have {}",
        tools::tool_names().len()
    );
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
        "unban_user",
        "timeout_member",
        "prune_members",
        "purge_messages",
        "delete_single_message",
        "delete_channel",
        "delete_role",
        "delete_emoji",
        "delete_invite",
        "delete_webhook",
        "delete_scheduled_event",
    ] {
        assert!(tools::is_destructive(name), "{name} should be destructive");
    }
    for name in [
        "list_channels",
        "channel_details",
        "user_info",
        "list_roles",
        "role_info",
        "search_members",
        "list_members",
        "server_info",
        "list_bans",
        "get_messages",
        "get_audit_logs",
        "send_message",
        "set_nickname",
        "remove_timeout",
        "warn_member",
        "create_role",
        "edit_role",
        "assign_role",
        "remove_role",
        "rename_channel",
        "move_channel",
        "clone_channel",
        "set_slowmode",
        "set_topic",
        "lock_channel",
        "create_channel",
        "create_thread",
        "create_invite",
        "create_webhook",
        "edit_server",
    ] {
        assert!(
            !tools::is_destructive(name),
            "{name} should not be destructive"
        );
    }
}
