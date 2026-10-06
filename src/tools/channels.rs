use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::{
    channel::ChannelType,
    id::{marker::GuildMarker, Id},
};

use super::helpers::{get_str, parse_channel_id};

fn channel_kind_label(kind: ChannelType) -> &'static str {
    match kind {
        ChannelType::GuildText => "text",
        ChannelType::GuildVoice => "voice",
        ChannelType::GuildAnnouncement => "announcement",
        ChannelType::GuildCategory => "category",
        _ => "other",
    }
}

/// JSON schema advertised to Gemini for `rename_channel`.
pub fn rename_schema() -> Value {
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

pub fn create_schema() -> Value {
    json!({
        "name": "create_channel",
        "description": "Creates a new text, voice, announcement, or category channel in the server.",
        "parameters": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name for the new channel (e.g. 'announcements')."
                },
                "kind": {
                    "type": "string",
                    "description": "One of: text, voice, announcement, category (default: text)."
                },
                "topic": {
                    "type": "string",
                    "description": "Optional channel topic (text channels only)."
                }
            },
            "required": ["name"]
        }
    })
}

pub fn delete_schema() -> Value {
    json!({
        "name": "delete_channel",
        "description": "Permanently deletes a channel from the server. Cannot be undone.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": {
                    "type": "string",
                    "description": "The Snowflake ID of the channel to delete."
                }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn slowmode_schema() -> Value {
    json!({
        "name": "set_slowmode",
        "description": "Sets slowmode on a text channel (users must wait N seconds between messages). Use 0 to disable.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": {
                    "type": "string",
                    "description": "The Snowflake ID of the channel."
                },
                "seconds": {
                    "type": "integer",
                    "description": "Seconds between messages per user (0-21600, default 0 = off)."
                }
            },
            "required": ["channel_id", "seconds"]
        }
    })
}

pub async fn rename(
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

pub async fn create(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let name = get_str(args, "name", "");
    if name.is_empty() {
        return Err("create_channel: 'name' must not be empty.".into());
    }
    let kind = match get_str(args, "kind", "text").to_lowercase().as_str() {
        "voice" => ChannelType::GuildVoice,
        "announcement" => ChannelType::GuildAnnouncement,
        "category" => ChannelType::GuildCategory,
        _ => ChannelType::GuildText,
    };
    let topic = get_str(args, "topic", "");

    let mut request = discord.create_guild_channel(guild_id, name).kind(kind);
    if !topic.is_empty() && kind == ChannelType::GuildText {
        request = request.topic(topic);
    }
    let channel = request.await?.model().await?;

    Ok(format!(
        "Successfully created {} channel '{}' with ID {}",
        channel_kind_label(channel.kind),
        channel.name.unwrap_or_else(|| name.to_string()),
        channel.id
    ))
}

pub async fn delete(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("delete_channel: {e}"))?;

    discord.delete_channel(channel_id).await?;

    Ok(format!("Successfully deleted channel {channel_id}"))
}

pub async fn slowmode(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("set_slowmode: {e}"))?;
    let seconds = args["seconds"].as_u64().unwrap_or(0).clamp(0, 21_600) as u16;

    discord
        .update_channel(channel_id)
        .rate_limit_per_user(seconds)
        .await?;

    if seconds == 0 {
        Ok(format!("Disabled slowmode in <#{channel_id}>"))
    } else {
        Ok(format!("Set slowmode to {seconds}s in <#{channel_id}>"))
    }
}
