use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use twilight_http::{request::AuditLogReason, Client as DiscordHttp};
use twilight_model::{
    channel::Message,
    id::{
        marker::{GuildMarker, MessageMarker},
        Id,
    },
    util::Timestamp,
};

use super::helpers::{clamp_int_arg, clamp_purge_count, get_str, parse_channel_id, parse_user_id};

/// JSON schema advertised to Gemini for `purge_messages`.
pub fn purge_schema() -> Value {
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

pub fn kick_schema() -> Value {
    json!({
        "name": "kick_member",
        "description": "Kicks a member from the server. They can rejoin with a new invite.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID (or @mention) of the member to kick."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional audit-log reason shown to moderators."
                }
            },
            "required": ["user_id"]
        }
    })
}

pub fn ban_schema() -> Value {
    json!({
        "name": "ban_member",
        "description": "Bans a member from the server. Optionally deletes their recent messages.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID (or @mention) of the member to ban."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional audit-log reason shown to moderators."
                },
                "delete_message_days": {
                    "type": "integer",
                    "description": "Days of the user's messages to delete (0-7, default 0)."
                }
            },
            "required": ["user_id"]
        }
    })
}

pub fn unban_schema() -> Value {
    json!({
        "name": "unban_user",
        "description": "Removes a ban so the user can join the server again.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID of the banned user."
                }
            },
            "required": ["user_id"]
        }
    })
}

pub fn timeout_schema() -> Value {
    json!({
        "name": "timeout_member",
        "description": "Times out a member (they cannot send messages or join voice) for a number of minutes.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID (or @mention) of the member to time out."
                },
                "minutes": {
                    "type": "integer",
                    "description": "Timeout duration in minutes (1-40320, i.e. up to 28 days)."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional audit-log reason shown to moderators."
                }
            },
            "required": ["user_id", "minutes"]
        }
    })
}

pub async fn purge(
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

    if total == 0 {
        return Ok(format!("No messages to purge in <#{channel_id}>"));
    }
    if total == 1 {
        discord.delete_message(channel_id, message_ids[0]).await?;
    } else {
        // Bulk delete rejects messages older than 14 days with a single
        // error; fall back to individual deletes so one old message doesn't
        // fail the whole purge.
        if let Err(e) = discord.delete_messages(channel_id, &message_ids).await {
            tracing::warn!(?e, "Bulk delete failed, falling back to single deletes");
            let mut deleted = 0usize;
            let mut failed = 0usize;
            for id in &message_ids {
                match discord.delete_message(channel_id, *id).await {
                    Ok(_) => deleted += 1,
                    Err(e) => {
                        tracing::warn!(?e, message_id = %id, "Single delete failed");
                        failed += 1;
                    }
                }
            }
            if deleted == 0 {
                return Err(format!(
                    "purge_messages: bulk delete failed ({e}) and all {failed} single deletes failed (messages may be older than 14 days or missing permissions)"
                )
                .into());
            }
            return Ok(format!(
                "Purged {deleted} messages in <#{channel_id}> ({failed} failed — likely older than 14 days)"
            ));
        }
    }

    Ok(format!(
        "Successfully purged {total} messages in <#{channel_id}>"
    ))
}

pub async fn kick(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("kick_member: {e}"))?;
    let reason = get_str(args, "reason", "");

    let request = discord.remove_guild_member(guild_id, user_id);
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }

    Ok(format!("Successfully kicked user {user_id}"))
}

pub async fn ban(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("ban_member: {e}"))?;
    let reason = get_str(args, "reason", "");
    let delete_days = clamp_int_arg(args, "delete_message_days", 0, 0, 7);

    let mut request = discord
        .create_ban(guild_id, user_id)
        .delete_message_seconds((delete_days * 86_400) as u32);
    if !reason.is_empty() {
        request = request.reason(reason);
    }
    request.await?;

    Ok(format!("Successfully banned user {user_id}"))
}

pub async fn unban(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("unban_user: {e}"))?;

    discord.delete_ban(guild_id, user_id).await?;

    Ok(format!("Successfully unbanned user {user_id}"))
}

pub async fn timeout(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("timeout_member: {e}"))?;
    let minutes = clamp_int_arg(args, "minutes", 10, 1, 40_320);
    let reason = get_str(args, "reason", "");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("timeout_member: clock error: {e}"))?
        .as_secs() as i64;
    let until = Timestamp::from_secs(now + (minutes as i64) * 60)
        .map_err(|e| format!("timeout_member: bad timestamp: {e}"))?;

    let request = discord
        .update_guild_member(guild_id, user_id)
        .communication_disabled_until(Some(until));
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }

    Ok(format!(
        "Successfully timed out user {user_id} for {minutes} minutes"
    ))
}
