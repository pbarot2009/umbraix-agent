use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::{
    channel::Message,
    id::{
        marker::{GuildMarker, MessageMarker},
        Id,
    },
};

use super::helpers::{clamp_purge_count, parse_channel_id};

/// JSON schema advertised to Gemini for `purge_messages`.
pub fn schema() -> Value {
    json!({
        "name": "purge_messages",
        "description": "Bulk deletes recent messages in a channel for moderation.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": {
                    "type": "string",
                    "description": "The Snowflake ID of the target channel."
                },
                "count": {
                    "type": "integer",
                    "description": "Number of messages to delete (between 1 and 100)."
                }
            },
            "required": ["channel_id", "count"]
        }
    })
}

pub async fn execute(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("purge_messages: {e}"))?;
    let count = clamp_purge_count(args);

    let messages: Vec<Message> = discord
        .channel_messages(channel_id)
        .limit(count)
        .await?
        .model()
        .await?;

    let message_ids: Vec<Id<MessageMarker>> = messages.into_iter().map(|m| m.id).collect();
    let total = message_ids.len();

    if total == 1 {
        discord.delete_message(channel_id, message_ids[0]).await?;
    } else if total > 1 {
        discord.delete_messages(channel_id, &message_ids).await?;
    }

    Ok(format!(
        "Successfully purged {total} messages in <#{channel_id}>"
    ))
}
