use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::{
    channel::ChannelType,
    id::{marker::GuildMarker, Id},
};

use super::helpers::{clamp_int_arg, get_str, parse_channel_id, parse_user_id, truncate_output};

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

pub fn list_active_threads_schema() -> Value {
    json!({
        "name": "list_active_threads",
        "description": "Lists all active (unarchived) threads in the server with IDs and parents. Check before joining/archiving.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn create_thread_schema() -> Value {
    json!({
        "name": "create_thread",
        "description": "Starts a public or private thread inside a text/announcement channel. Threads keep side discussions organized.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Parent text/announcement channel." },
                "name": { "type": "string", "description": "Thread name (1-100 chars)." },
                "kind": { "type": "string", "description": "Either 'public' (default) or 'private'." },
                "auto_archive_minutes": { "type": "integer", "description": "Auto-archive: 60, 1440, 4320 or 10080 (default 1440)." }
            },
            "required": ["channel_id", "name"]
        }
    })
}

pub fn thread_from_message_schema() -> Value {
    json!({
        "name": "thread_from_message",
        "description": "Starts a thread attached to an existing message (great for Q&A follow-ups).",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel containing the message." },
                "message_id": { "type": "string", "description": "Message to branch from." },
                "name": { "type": "string", "description": "Thread name (1-100 chars)." }
            },
            "required": ["channel_id", "message_id", "name"]
        }
    })
}

pub fn join_thread_schema() -> Value {
    json!({
        "name": "join_thread",
        "description": "Makes the bot join a thread so it can read and post there.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Thread channel ID." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn archive_thread_schema() -> Value {
    json!({
        "name": "archive_thread",
        "description": "Archives (or unarchives) a thread. Archived threads are read-only but preserved.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Thread channel ID." },
                "archived": { "type": "boolean", "description": "true = archive, false = reopen." },
                "locked": { "type": "boolean", "description": "Optionally lock/unlock alongside." }
            },
            "required": ["channel_id", "archived"]
        }
    })
}

pub fn list_thread_members_schema() -> Value {
    json!({
        "name": "list_thread_members",
        "description": "Lists members who joined a thread.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Thread channel ID." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn add_thread_member_schema() -> Value {
    json!({
        "name": "add_thread_member",
        "description": "Adds a member to a thread.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Thread channel ID." },
                "user_id": { "type": "string", "description": "Member to add." }
            },
            "required": ["channel_id", "user_id"]
        }
    })
}

pub fn remove_thread_member_schema() -> Value {
    json!({
        "name": "remove_thread_member",
        "description": "Removes a member from a thread.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Thread channel ID." },
                "user_id": { "type": "string", "description": "Member to remove." }
            },
            "required": ["channel_id", "user_id"]
        }
    })
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

pub async fn list_active_threads(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let res = discord.active_threads(guild_id).await?.model().await?;
    if res.threads.is_empty() {
        return Ok("No active threads.".to_string());
    }
    let mut lines = Vec::new();
    for t in res.threads.iter().take(50) {
        lines.push(format!(
            "- {} id={} parent={:?}",
            t.name.as_deref().unwrap_or("(no name)"),
            t.id,
            t.parent_id,
        ));
    }
    Ok(truncate_output(
        format!("Active threads ({}):\n{}", res.threads.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn create_thread(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_model::channel::thread::AutoArchiveDuration;
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("create_thread: {e}"))?;
    let name = get_str(args, "name", "");
    if name.is_empty() || name.len() > 100 {
        return Err("create_thread: 'name' must be 1-100 characters.".into());
    }
    let kind = match get_str(args, "kind", "public").to_lowercase().as_str() {
        "private" => ChannelType::PrivateThread,
        _ => ChannelType::PublicThread,
    };
    let auto_archive = match clamp_int_arg(args, "auto_archive_minutes", 1440, 60, 10080) {
        60 => AutoArchiveDuration::Hour,
        4320 => AutoArchiveDuration::ThreeDays,
        10080 => AutoArchiveDuration::Week,
        _ => AutoArchiveDuration::Day,
    };
    let thread = discord
        .create_thread(channel_id, name, kind)
        .auto_archive_duration(auto_archive)
        .await?
        .model()
        .await?;
    Ok(format!(
        "Created thread '{}' (id={}) in <#{channel_id}>",
        thread.name.as_deref().unwrap_or(name),
        thread.id
    ))
}

pub async fn thread_from_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("thread_from_message: {e}"))?;
    let message_id = if let Some(s) = args["message_id"].as_str() {
        s.trim().parse::<u64>().map_err(|_| "thread_from_message: invalid message_id.")?
    } else if let Some(n) = args["message_id"].as_u64() {
        n
    } else {
        return Err("thread_from_message: 'message_id' is required.".into());
    };
    let name = get_str(args, "name", "");
    if name.is_empty() || name.len() > 100 {
        return Err("thread_from_message: 'name' must be 1-100 characters.".into());
    }
    let thread = discord
        .create_thread_from_message(
            channel_id,
            twilight_model::id::Id::new(message_id),
            name,
        )
        .await?
        .model()
        .await?;
    Ok(format!(
        "Created thread '{}' (id={}) from message {message_id}",
        thread.name.as_deref().unwrap_or(name),
        thread.id
    ))
}

pub async fn join_thread(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("join_thread: {e}"))?;
    discord.join_thread(channel_id).await?;
    Ok(format!("Joined thread <#{channel_id}>"))
}

pub async fn archive_thread(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("archive_thread: {e}"))?;
    let archived = args
        .get("archived")
        .and_then(|v| v.as_bool())
        .ok_or("archive_thread: 'archived' must be true or false.")?;
    let mut req = discord.update_thread(channel_id).archived(archived);
    if let Some(locked) = args.get("locked").and_then(|v| v.as_bool()) {
        req = req.locked(locked);
    }
    req.await?;
    Ok(format!(
        "{} thread <#{channel_id}>",
        if archived { "Archived" } else { "Reopened" }
    ))
}

pub async fn list_thread_members(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("list_thread_members: {e}"))?;
    let members = discord.thread_members(channel_id).await?.model().await?;
    if members.is_empty() {
        return Ok(format!("No members in thread <#{channel_id}>."));
    }
    let lines: Vec<String> = members
        .iter()
        .take(50)
        .map(|m| format!("- user={:?} joined={}", m.user_id, m.join_timestamp.iso_8601()))
        .collect();
    Ok(truncate_output(
        format!("Thread <#{channel_id}> members ({}):\n{}", members.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn add_thread_member(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("add_thread_member: {e}"))?;
    let user_id =
        parse_user_id(args, "user_id").map_err(|e| format!("add_thread_member: {e}"))?;
    discord.add_thread_member(channel_id, user_id).await?;
    Ok(format!("Added <@{user_id}> to thread <#{channel_id}>"))
}

pub async fn remove_thread_member(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("remove_thread_member: {e}"))?;
    let user_id =
        parse_user_id(args, "user_id").map_err(|e| format!("remove_thread_member: {e}"))?;
    discord.remove_thread_member(channel_id, user_id).await?;
    Ok(format!("Removed <@{user_id}> from thread <#{channel_id}>"))
}
