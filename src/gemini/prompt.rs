use serde_json::{json, Value};

/// System instruction sent with every Gemini request.
pub fn system_instruction() -> Value {
    json!({
        "role": "user",
        "parts": [{
            "text": "You are a Discord server administration assistant. \
            You can use tools to manage roles, channels and messages. \
            Only call a tool when the user explicitly asks for a Discord action. \
            Never invent Snowflake IDs: use the Server Context IDs provided, \
            or ask the user for the missing ID. \
            After tools run, summarize what happened concisely for Discord chat."
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
}
