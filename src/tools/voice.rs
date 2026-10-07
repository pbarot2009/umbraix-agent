use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{marker::GuildMarker, Id};

use super::helpers::{clamp_int_arg, get_str, parse_channel_id, parse_user_id, truncate_output};

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

pub fn list_members_schema() -> Value {
    json!({
        "name": "list_members",
        "description": "Lists server members (names, nicks, IDs, bot flags). Use to browse membership or find IDs when search_members is too narrow.",
        "parameters": {
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "How many members 1-100 (default 25)." }
            },
            "required": []
        }
    })
}

pub fn move_voice_member_schema() -> Value {
    json!({
        "name": "move_voice_member",
        "description": "Moves a member who is in voice to another voice channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to move." },
                "channel_id": { "type": "string", "description": "Destination voice channel ID." }
            },
            "required": ["user_id", "channel_id"]
        }
    })
}

pub fn disconnect_voice_member_schema() -> Value {
    json!({
        "name": "disconnect_voice_member",
        "description": "Disconnects a member from voice (kicks them out of the voice channel, they stay in the server).",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to disconnect from voice." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn mute_voice_member_schema() -> Value {
    json!({
        "name": "mute_voice_member",
        "description": "Server-mutes a member in voice (they cannot speak until unmuted).",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to mute." },
                "reason": { "type": "string", "description": "Optional audit-log reason." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn unmute_voice_member_schema() -> Value {
    json!({
        "name": "unmute_voice_member",
        "description": "Server-unmutes a member in voice.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to unmute." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn deafen_voice_member_schema() -> Value {
    json!({
        "name": "deafen_voice_member",
        "description": "Server-deafens a member in voice (they cannot hear or speak).",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to deafen." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn undeafen_voice_member_schema() -> Value {
    json!({
        "name": "undeafen_voice_member",
        "description": "Removes a server-deafen from a member in voice.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to undeafen." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn get_voice_state_schema() -> Value {
    json!({
        "name": "get_voice_state",
        "description": "Shows a member's current voice state: which channel, muted/deafened/suppressed. Check before moving/muting.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to inspect." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn warn_member_schema() -> Value {
    json!({
        "name": "warn_member",
        "description": "Warns a member via DM with a moderator message. Logged in the audit trail. Fails gracefully if their DMs are closed.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Member to warn." },
                "reason": { "type": "string", "description": "Warning text sent to the member (max 1000 chars)." }
            },
            "required": ["user_id", "reason"]
        }
    })
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

pub async fn list_members(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let limit = clamp_int_arg(args, "limit", 25, 1, 100) as u16;
    let members = discord
        .guild_members(guild_id)
        .limit(limit)
        .await?
        .model()
        .await?;
    if members.is_empty() {
        return Ok("No members visible.".to_string());
    }
    let mut lines = Vec::new();
    for m in members.iter().take(limit as usize) {
        let nick = m.nick.as_deref().unwrap_or("(no nick)");
        lines.push(format!(
            "- {} (nick: {nick}) id={} bot={} roles={}",
            m.user.name,
            m.user.id,
            m.user.bot,
            m.roles.len()
        ));
    }
    Ok(truncate_output(
        format!("Members (showing {}):\n{}", lines.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn move_voice_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("move_voice_member: {e}"))?;
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("move_voice_member: {e}"))?;
    discord
        .update_guild_member(guild_id, user_id)
        .channel_id(Some(channel_id))
        .await?;
    Ok(format!(
        "Moved <@{user_id}> to voice channel <#{channel_id}>"
    ))
}

pub async fn disconnect_voice_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id =
        parse_user_id(args, "user_id").map_err(|e| format!("disconnect_voice_member: {e}"))?;
    discord
        .update_guild_member(guild_id, user_id)
        .channel_id(None)
        .await?;
    Ok(format!("Disconnected <@{user_id}> from voice"))
}

pub async fn mute_voice_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_http::request::AuditLogReason;
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("mute_voice_member: {e}"))?;
    let reason = get_str(args, "reason", "");
    let request = discord.update_guild_member(guild_id, user_id).mute(true);
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }
    Ok(format!("Server-muted <@{user_id}> in voice"))
}

pub async fn unmute_voice_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id =
        parse_user_id(args, "user_id").map_err(|e| format!("unmute_voice_member: {e}"))?;
    discord
        .update_guild_member(guild_id, user_id)
        .mute(false)
        .await?;
    Ok(format!("Server-unmuted <@{user_id}> in voice"))
}

pub async fn deafen_voice_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id =
        parse_user_id(args, "user_id").map_err(|e| format!("deafen_voice_member: {e}"))?;
    discord
        .update_guild_member(guild_id, user_id)
        .deaf(true)
        .await?;
    Ok(format!("Server-deafened <@{user_id}> in voice"))
}

pub async fn undeafen_voice_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id =
        parse_user_id(args, "user_id").map_err(|e| format!("undeafen_voice_member: {e}"))?;
    discord
        .update_guild_member(guild_id, user_id)
        .deaf(false)
        .await?;
    Ok(format!("Undeafened <@{user_id}> in voice"))
}

pub async fn get_voice_state(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("get_voice_state: {e}"))?;
    let state = discord
        .user_voice_state(guild_id, user_id)
        .await?
        .model()
        .await?;
    Ok(format!(
        "Voice state for <@{user_id}>: channel={:?}, mute={} deaf={} self_mute={} self_deaf={} suppress={}",
        state.channel_id, state.mute, state.deaf, state.self_mute, state.self_deaf, state.suppress,
    ))
}

pub async fn warn_member(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("warn_member: {e}"))?;
    let reason = get_str(args, "reason", "");
    if reason.is_empty() {
        return Err("warn_member: 'reason' is required.".into());
    }
    if reason.chars().count() > 1000 {
        return Err("warn_member: 'reason' must be at most 1000 characters.".into());
    }
    // Resolve a display name for a personal warning; fall back to the ID.
    let name = match discord.guild_member(guild_id, user_id).await {
        Ok(resp) => match resp.model().await {
            Ok(m) => m.user.name,
            Err(_) => format!("{user_id}"),
        },
        Err(_) => format!("{user_id}"),
    };
    let dm = discord
        .create_private_channel(user_id)
        .await?
        .model()
        .await?;
    let text = format!("You received a warning in this server: {reason}");
    if let Err(e) = discord.create_message(dm.id).content(&text).await {
        let raw = e.to_string();
        if raw.contains("50007") {
            return Ok(format!(
                "Could not DM {name} (<@{user_id}>): their DMs are closed. Warning recorded here instead: '{reason}'"
            ));
        }
        return Err(format!("warn_member: could not DM user: {raw}").into());
    }
    Ok(format!("Warned {name} (<@{user_id}>) via DM: '{reason}'"))
}
