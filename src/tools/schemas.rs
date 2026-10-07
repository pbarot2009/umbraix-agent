use serde_json::{json, Value};

use super::{
    channels, extra, info, manage_channels, manage_roles, messages, moderation, roles, server,
    threads, voice,
};

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
            messages::channel_details_schema(),
            threads::list_active_threads_schema(),
            info::user_info_schema(),
            extra::list_roles_schema(),
            manage_roles::role_info_schema(),
            manage_roles::list_role_members_schema(),
            extra::search_members_schema(),
            voice::list_members_schema(),
            extra::server_info_schema(),
            extra::list_bans_schema(),
            extra::get_messages_schema(),
            messages::get_message_details_schema(),
            server::get_audit_logs_schema(),
            server::list_emojis_schema(),
            server::list_invites_schema(),
            server::list_channel_invites_schema(),
            messages::list_pins_schema(),
            server::list_webhooks_schema(),
            server::list_scheduled_events_schema(),
            server::list_automod_rules_schema(),
            threads::list_thread_members_schema(),
            voice::get_voice_state_schema(),
            server::prune_preview_schema(),
            // Messaging.
            extra::send_message_schema(),
            messages::edit_message_schema(),
            messages::delete_single_message_schema(),
            messages::pin_message_schema(),
            messages::unpin_message_schema(),
            messages::publish_message_schema(),
            // Moderation.
            moderation::kick_schema(),
            moderation::ban_schema(),
            moderation::unban_schema(),
            moderation::timeout_schema(),
            extra::remove_timeout_schema(),
            moderation::purge_schema(),
            extra::set_nickname_schema(),
            voice::warn_member_schema(),
            server::prune_members_schema(),
            // Channels & categories.
            channels::create_schema(),
            channels::rename_schema(),
            channels::delete_schema(),
            manage_channels::move_channel_schema(),
            manage_channels::clone_channel_schema(),
            channels::slowmode_schema(),
            extra::set_topic_schema(),
            manage_channels::set_channel_nsfw_schema(),
            manage_channels::set_voice_limits_schema(),
            manage_channels::lock_channel_schema(),
            manage_channels::unlock_channel_schema(),
            manage_channels::set_channel_permissions_schema(),
            manage_channels::clear_channel_permissions_schema(),
            // Roles.
            roles::create_schema(),
            manage_roles::edit_role_schema(),
            manage_roles::set_role_color_schema(),
            manage_roles::set_role_permissions_schema(),
            manage_roles::set_role_position_schema(),
            roles::assign_schema(),
            roles::remove_schema(),
            extra::delete_role_schema(),
            // Voice.
            voice::move_voice_member_schema(),
            voice::disconnect_voice_member_schema(),
            voice::mute_voice_member_schema(),
            voice::unmute_voice_member_schema(),
            voice::deafen_voice_member_schema(),
            voice::undeafen_voice_member_schema(),
            // Threads.
            threads::create_thread_schema(),
            threads::thread_from_message_schema(),
            threads::join_thread_schema(),
            threads::archive_thread_schema(),
            threads::add_thread_member_schema(),
            threads::remove_thread_member_schema(),
            // Invites / webhooks / emojis / events / server.
            server::create_invite_schema(),
            server::delete_invite_schema(),
            server::create_webhook_schema(),
            server::delete_webhook_schema(),
            server::rename_emoji_schema(),
            server::delete_emoji_schema(),
            server::delete_scheduled_event_schema(),
            server::edit_server_schema(),
        ]
    }])
}

/// Names of all registered tools (used by tests and diagnostics).
pub fn tool_names() -> &'static [&'static str] {
    &[
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
    ]
}

/// Whether a tool performs a destructive or hard-to-reverse Discord action.
///
/// Used by the ReAct loop for extra logging; a human confirmation gate
/// can be wired to this in a later phase.
pub fn is_destructive(name: &str) -> bool {
    matches!(
        name,
        "kick_member"
            | "ban_member"
            | "unban_user"
            | "timeout_member"
            | "prune_members"
            | "purge_messages"
            | "delete_single_message"
            | "delete_channel"
            | "delete_role"
            | "delete_emoji"
            | "delete_invite"
            | "delete_webhook"
            | "delete_scheduled_event"
            | "edit_server"
            | "set_role_permissions"
            | "set_channel_permissions"
            | "clear_channel_permissions"
            | "create_channel"
            | "lock_channel"
    )
}

/// Tools that only read state; safe to run concurrently within one step.
pub fn is_read_only(name: &str) -> bool {
    matches!(
        name,
        "list_channels"
            | "channel_details"
            | "list_active_threads"
            | "user_info"
            | "list_roles"
            | "role_info"
            | "list_role_members"
            | "search_members"
            | "list_members"
            | "server_info"
            | "list_bans"
            | "get_messages"
            | "get_message_details"
            | "get_audit_logs"
            | "list_emojis"
            | "list_invites"
            | "list_channel_invites"
            | "list_pins"
            | "list_webhooks"
            | "list_scheduled_events"
            | "list_automod_rules"
            | "list_thread_members"
            | "get_voice_state"
            | "prune_preview"
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

    #[test]
    fn catalog_has_full_coverage() {
        assert!(
            tool_names().len() >= 60,
            "need 60+ tools, have {}",
            tool_names().len()
        );
        for must in [
            "move_channel",
            "clone_channel",
            "lock_channel",
            "set_channel_permissions",
            "edit_role",
            "set_role_position",
            "move_voice_member",
            "create_thread",
            "create_invite",
            "create_webhook",
            "edit_server",
            "get_audit_logs",
        ] {
            assert!(tool_names().contains(&must), "missing flagship tool {must}");
        }
    }
}
