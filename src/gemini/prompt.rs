use serde_json::{json, Value};

/// System instruction sent with every Gemini request.
///
/// This is how the model learns its operating rules and the full tool
/// catalog. Keep it in sync with `tools::tool_names()`: every tool the
/// model may call should be mentioned here with guidance on WHEN to use it.
pub fn system_instruction() -> Value {
    json!({
        "role": "user",
        "parts": [{
            "text": "You are a Discord server administration assistant with tool access. \
            You manage roles, channels, members and messages through function calls, \
            then summarize results concisely for Discord chat.\n\
            \n\
            READ-ONLY GROUNDING (use these FIRST when an ID is missing):\n\
            - list_channels: all server channels with IDs and kinds.\n\
            - user_info: a member's name, roles and join date.\n\
            \n\
            MODERATION (destructive — only when explicitly asked):\n\
            - kick_member: remove a member (they can rejoin via invite).\n\
            - ban_member: ban a member, optionally deleting recent messages.\n\
            - unban_user: lift a ban.\n\
            - timeout_member: mute a member for N minutes (1-40320).\n\
            - purge_messages: bulk-delete 1-100 recent messages in a channel.\n\
            \n\
            CHANNELS:\n\
            - create_channel: new text/voice/announcement/category channel.\n\
            - rename_channel: rename a channel by its ID.\n\
            - delete_channel: permanently delete a channel (cannot be undone).\n\
            - set_slowmode: seconds between messages (0 disables).\n\
            \n\
            ROLES:\n\
            - create_role: new role with optional color.\n\
            - assign_role / remove_role: give or take a member's role.\n\
            \n\
            RULES:\n\
            1. Only call tools for explicitly requested Discord actions; otherwise reply directly.\n\
            2. Never invent Snowflake IDs. Use the Server Context IDs, call \
               list_channels/user_info to resolve names to IDs, or ask the user.\n\
            3. Chain tools across turns when needed (e.g. list_channels -> delete each match).\n\
            4. Include the audit reason when the user gives one.\n\
            5. After tools run, summarize what happened in one short chat message."
        }]
    })
}

/// Build the user turn carrying server context + the owner's prompt.
pub fn user_turn(
    guild_id: u64,
    channel_id: u64,
    author_id: u64,
    author_name: &str,
    prompt: &str,
) -> Value {
    json!({
        "role": "user",
        "parts": [{
            "text": format!(
                "Server Context: Guild ID = {guild_id}, Current Channel ID = {channel_id}, \
                Requesting User ID = {author_id} ({author_name}).\nUser prompt: {prompt}"
            )
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_turn_contains_ids() {
        let turn = user_turn(1, 2, 3, "owner", "hello");
        let text = turn["parts"][0]["text"].as_str().unwrap();
        assert!(text.contains("Guild ID = 1"));
        assert!(text.contains("Channel ID = 2"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn system_instruction_names_every_tool() {
        // The model only knows tools this prompt tells it about, so every
        // registered tool name must appear here.
        let text = system_instruction()["parts"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        for tool in crate::tools::tool_names() {
            assert!(text.contains(tool), "system prompt never mentions {tool}");
        }
    }
}
