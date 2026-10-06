use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{marker::GuildMarker, Id};

use super::helpers::{get_str, parse_channel_id};

/// JSON schema advertised to Gemini for `rename_channel`.
pub fn schema() -> Value {
    json!({
        "name": "rename_channel",
        "description": "Renames a specific Discord channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": {
                    "type": "string",
                    "description": "The Snowflake ID of the channel to rename."
                },
                "new_name": {
                    "type": "string",
                    "description": "The new sanitized name for the channel."
                }
            },
            "required": ["channel_id", "new_name"]
        }
    })
}

pub async fn execute(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("rename_channel: {e}"))?;
    let new_name = get_str(args, "new_name", "channel");
    if new_name.is_empty() {
        return Err("rename_channel: 'new_name' must not be empty.".into());
    }

    discord.update_channel(channel_id).name(new_name).await?;

    Ok(format!(
        "Successfully renamed channel <#{channel_id}> to '{new_name}'"
    ))
}
