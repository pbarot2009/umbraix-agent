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
    // Never fall back to a placeholder name: a missing arg must fail loudly
    // instead of renaming a channel to "channel".
    let new_name = get_str(args, "new_name", "");
    if new_name.is_empty() {
        return Err("rename_channel: 'new_name' is required and must not be empty.".into());
    }
    if new_name.len() > 100 {
        return Err("rename_channel: 'new_name' must be at most 100 characters.".into());
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
        return Err("create_channel: 'name' is required and must not be empty.".into());
    }
    if name.chars().count() > 100 {
        return Err("create_channel: 'name' must be at most 100 characters.".into());
    }
    let kind = match get_str(args, "kind", "text").to_lowercase().as_str() {
        "text" => ChannelType::GuildText,
        "voice" => ChannelType::GuildVoice,
        "announcement" => ChannelType::GuildAnnouncement,
        "category" => ChannelType::GuildCategory,
        other => {
            return Err(format!(
                "create_channel: unknown kind '{other}' (want text/voice/announcement/category)."
            )
            .into())
        }
    };
    let topic = get_str(args, "topic", "");

    let mut request = discord.create_guild_channel(guild_id, name).kind(kind);
    // Topics are valid on text AND announcement channels; silently dropping
    // them for announcements wasted a step and confused the model.
    if !topic.is_empty()
        && (kind == ChannelType::GuildText || kind == ChannelType::GuildAnnouncement)
    {
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
    // Strict: out-of-range is a model error, not a silent rewrite.
    let raw_seconds = args.get("seconds").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str()?.trim().parse::<u64>().ok())
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
    });
    let Some(seconds_raw) = raw_seconds else {
        return Err("set_slowmode: 'seconds' must be an integer 0-21600.".into());
    };
    if seconds_raw > 21_600 {
        return Err("set_slowmode: 'seconds' must be 0-21600.".into());
    }
    let seconds = seconds_raw as u16;

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
