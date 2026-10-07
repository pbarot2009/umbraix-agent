use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, MessageMarker},
    Id,
};

use super::helpers::{get_str, parse_channel_id, truncate_output};

fn parse_message_id(args: &Value, field: &str) -> Result<Id<MessageMarker>, String> {
    if let Some(s) = args[field].as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Err(format!("Missing required field '{field}'."));
        }
        return s
            .parse::<u64>()
            .map(Id::new)
            .map_err(|_| format!("Invalid message ID '{s}'."));
    }
    if let Some(n) = args[field].as_u64() {
        if n == 0 {
            return Err(format!("Missing required field '{field}'."));
        }
        return Ok(Id::new(n));
    }
    Err(format!("Missing required field '{field}'."))
}

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

pub fn channel_details_schema() -> Value {
    json!({
        "name": "channel_details",
        "description": "Shows full details of one channel: name, kind, topic, NSFW, slowmode, parent category, position, voice bitrate/user limit. Call before editing/moving a channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Snowflake ID (or <#...> mention) of the channel." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn get_message_details_schema() -> Value {
    json!({
        "name": "get_message_details",
        "description": "Fetches one message by ID: author, content, timestamp, embeds, attachments, pinned flag. Use to inspect before edit/pin/delete.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel containing the message." },
                "message_id": { "type": "string", "description": "Snowflake ID of the message." }
            },
            "required": ["channel_id", "message_id"]
        }
    })
}

pub fn edit_message_schema() -> Value {
    json!({
        "name": "edit_message",
        "description": "Edits the text content of a message the bot sent (or any message it can edit). Only content is changed.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel containing the message." },
                "message_id": { "type": "string", "description": "Message to edit." },
                "content": { "type": "string", "description": "New text (1-2000 chars)." }
            },
            "required": ["channel_id", "message_id", "content"]
        }
    })
}

pub fn delete_single_message_schema() -> Value {
    json!({
        "name": "delete_single_message",
        "description": "Deletes exactly one message by ID. For bulk cleanup use purge_messages instead.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel containing the message." },
                "message_id": { "type": "string", "description": "Message to delete." }
            },
            "required": ["channel_id", "message_id"]
        }
    })
}

pub fn pin_message_schema() -> Value {
    json!({
        "name": "pin_message",
        "description": "Pins a message to the top of its channel (max 50 pins per channel).",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel containing the message." },
                "message_id": { "type": "string", "description": "Message to pin." }
            },
            "required": ["channel_id", "message_id"]
        }
    })
}

pub fn unpin_message_schema() -> Value {
    json!({
        "name": "unpin_message",
        "description": "Unpins a message from its channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel containing the message." },
                "message_id": { "type": "string", "description": "Message to unpin." }
            },
            "required": ["channel_id", "message_id"]
        }
    })
}

pub fn list_pins_schema() -> Value {
    json!({
        "name": "list_pins",
        "description": "Lists all pinned messages in a channel with authors and IDs. Check before pinning (50-pin cap).",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to list pins from." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn publish_message_schema() -> Value {
    json!({
        "name": "publish_message",
        "description": "Publishes (crossposts) a message from an announcement channel to all following channels.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Announcement channel containing the message." },
                "message_id": { "type": "string", "description": "Message to publish." }
            },
            "required": ["channel_id", "message_id"]
        }
    })
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

pub async fn channel_details(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("channel_details: {e}"))?;
    let c = discord.channel(channel_id).await?.model().await?;
    Ok(format!(
        "Channel '{}' (id={}): kind={:?}, topic='{}', nsfw={}, slowmode={}s, position={:?}, parent={:?}, bitrate={:?}, user_limit={:?}",
        c.name.as_deref().unwrap_or("(no name)"),
        c.id,
        c.kind,
        c.topic.as_deref().unwrap_or("(none)"),
        c.nsfw.unwrap_or(false),
        c.rate_limit_per_user.unwrap_or(0),
        c.position,
        c.parent_id,
        c.bitrate,
        c.user_limit,
    ))
}

pub async fn get_message_details(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("get_message_details: {e}"))?;
    let message_id =
        parse_message_id(args, "message_id").map_err(|e| format!("get_message_details: {e}"))?;
    let m = discord
        .message(channel_id, message_id)
        .await?
        .model()
        .await?;
    let content: String = m.content.chars().take(500).collect();
    Ok(format!(
        "Message {message_id} in <#{channel_id}> by {} (id={}): '{}' pinned={} embeds={} attachments={} tts={}",
        m.author.name,
        m.author.id,
        content.replace('\n', " "),
        m.pinned,
        m.embeds.len(),
        m.attachments.len(),
        m.tts,
    ))
}

pub async fn edit_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id: Id<ChannelMarker> =
        parse_channel_id(args, "channel_id").map_err(|e| format!("edit_message: {e}"))?;
    let message_id =
        parse_message_id(args, "message_id").map_err(|e| format!("edit_message: {e}"))?;
    let content = get_str(args, "content", "");
    if content.is_empty() {
        return Err("edit_message: 'content' is required and must not be empty.".into());
    }
    if content.chars().count() > 2000 {
        return Err("edit_message: 'content' must be at most 2000 characters.".into());
    }
    discord
        .update_message(channel_id, message_id)
        .content(Some(content))
        .await?;
    Ok(format!("Edited message {message_id} in <#{channel_id}>"))
}

pub async fn delete_single_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("delete_single_message: {e}"))?;
    let message_id =
        parse_message_id(args, "message_id").map_err(|e| format!("delete_single_message: {e}"))?;
    discord.delete_message(channel_id, message_id).await?;
    Ok(format!("Deleted message {message_id} from <#{channel_id}>"))
}

pub async fn pin_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("pin_message: {e}"))?;
    let message_id =
        parse_message_id(args, "message_id").map_err(|e| format!("pin_message: {e}"))?;
    discord.create_pin(channel_id, message_id).await?;
    Ok(format!("Pinned message {message_id} in <#{channel_id}>"))
}

pub async fn unpin_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("unpin_message: {e}"))?;
    let message_id =
        parse_message_id(args, "message_id").map_err(|e| format!("unpin_message: {e}"))?;
    discord.delete_pin(channel_id, message_id).await?;
    Ok(format!("Unpinned message {message_id} in <#{channel_id}>"))
}

pub async fn list_pins(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id = parse_channel_id(args, "channel_id").map_err(|e| format!("list_pins: {e}"))?;
    let pins = discord.pins(channel_id).await?.model().await?;
    if pins.items.is_empty() {
        return Ok(format!("No pinned messages in <#{channel_id}>."));
    }
    let mut lines = Vec::new();
    for p in pins.items.iter().take(50) {
        let m = &p.message;
        let content: String = m.content.chars().take(120).collect();
        lines.push(format!(
            "- {} by {}: {} (msg_id={})",
            m.id,
            m.author.name,
            content.replace('\n', " "),
            m.id
        ));
    }
    Ok(truncate_output(
        format!(
            "Pinned in <#{channel_id}> ({}):\n{}",
            pins.items.len(),
            lines.join("\n")
        ),
        4000,
    ))
}

pub async fn publish_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("publish_message: {e}"))?;
    let message_id =
        parse_message_id(args, "message_id").map_err(|e| format!("publish_message: {e}"))?;
    discord.crosspost_message(channel_id, message_id).await?;
    Ok(format!(
        "Published message {message_id} from <#{channel_id}> to following channels"
    ))
}
