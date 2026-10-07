use serde_json::{json, Value};
use twilight_http::{request::AuditLogReason, Client as DiscordHttp};
use twilight_model::id::{marker::GuildMarker, Id};

use super::helpers::truncate_output;
use super::helpers::{clamp_int_arg, get_str, parse_channel_id, parse_role_id, parse_user_id};

// ---------------------------------------------------------------------------
// 10 new tools closing real-world gaps:
//
// Grounding (the model previously had to hallucinate IDs):
//  1. list_roles      — resolve role names -> IDs (assign/remove were unusable)
//  2. search_members  — resolve user names -> IDs (user_info needed an ID)
//  3. server_info     — guild overview (member count, owner, boosts)
//  4. list_bans       — grounding for unban_user (previously no way to list bans)
//
// Observability / messaging:
//  5. get_messages    — read recent history before purge/moderation
//  6. send_message    — speak in another channel (bot could only reply inline)
//
// Lifecycle completeness:
//  7. set_nickname    — basic moderation primitive that was missing
//  8. delete_role     — create_role without delete leaked roles
//  9. remove_timeout  — timeout without untimeout was one-way
// 10. set_topic       — topic settable at create but never editable
// ---------------------------------------------------------------------------

pub fn list_roles_schema() -> Value {
    json!({
        "name": "list_roles",
        "description": "Lists all roles in the server with IDs, names, colors and member counts. Call this first to resolve a role NAME to its ID before assign_role/remove_role/delete_role.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn search_members_schema() -> Value {
    json!({
        "name": "search_members",
        "description": "Searches server members by name or nickname and returns their user IDs. Call this first when a request names a user but gives no ID or @mention.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Username or nickname to search for (e.g. 'prathmesh')." },
                "limit": { "type": "integer", "description": "Max results 1-20 (default 10)." }
            },
            "required": ["query"]
        }
    })
}

pub fn server_info_schema() -> Value {
    json!({
        "name": "server_info",
        "description": "Shows server overview: name, ID, owner, member counts, boost level, creation date. Use for 'how big is this server' questions and pre-action context.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn list_bans_schema() -> Value {
    json!({
        "name": "list_bans",
        "description": "Lists banned users with IDs and reasons. Call this first to resolve WHO is banned before unban_user, or to answer 'who is banned?'.",
        "parameters": {
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Max entries 1-50 (default 25)." }
            },
            "required": []
        }
    })
}

pub fn get_messages_schema() -> Value {
    json!({
        "name": "get_messages",
        "description": "Reads recent messages in a channel (author + content + IDs) WITHOUT deleting. Use to inspect context before purge_messages or to summarize discussion.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Snowflake ID (or <#...> mention) of the channel to read." },
                "count": { "type": "integer", "description": "Messages to read 1-50 (default 10)." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn send_message_schema() -> Value {
    json!({
        "name": "send_message",
        "description": "Sends a message to a specific channel. Use for announcements, cross-posting, or welcome messages outside the current channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Snowflake ID (or <#...> mention) of the destination channel." },
                "content": { "type": "string", "description": "Message text (max 2000 chars)." }
            },
            "required": ["channel_id", "content"]
        }
    })
}

pub fn set_nickname_schema() -> Value {
    json!({
        "name": "set_nickname",
        "description": "Sets or clears a member's server nickname. Pass empty string, 'clear' or 'reset' to remove the nickname.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Snowflake ID (or @mention) of the member." },
                "nickname": { "type": "string", "description": "New nickname (1-32 chars), or empty/'clear' to remove." },
                "reason": { "type": "string", "description": "Optional audit-log reason." }
            },
            "required": ["user_id", "nickname"]
        }
    })
}

pub fn delete_role_schema() -> Value {
    json!({
        "name": "delete_role",
        "description": "Permanently deletes a role from the server. Members keep no trace of it. Resolve the name via list_roles first.",
        "parameters": {
            "type": "object",
            "properties": {
                "role_id": { "type": "string", "description": "Snowflake ID (or <@&...> mention) of the role to delete." }
            },
            "required": ["role_id"]
        }
    })
}

pub fn remove_timeout_schema() -> Value {
    json!({
        "name": "remove_timeout",
        "description": "Removes a member's timeout early (they can speak again immediately). Use when a mute was a mistake or has served its purpose.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "Snowflake ID (or @mention) of the member." },
                "reason": { "type": "string", "description": "Optional audit-log reason." }
            },
            "required": ["user_id"]
        }
    })
}

pub fn set_topic_schema() -> Value {
    json!({
        "name": "set_topic",
        "description": "Sets a text or announcement channel's topic (shown at the top). Pass empty string to clear.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Snowflake ID of the channel." },
                "topic": { "type": "string", "description": "New topic (0-1024 chars; empty clears)." }
            },
            "required": ["channel_id", "topic"]
        }
    })
}

pub async fn list_roles(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let roles = discord.roles(guild_id).await?.model().await?;
    if roles.is_empty() {
        return Ok("The server has no roles besides @everyone.".to_string());
    }
    let mut lines = Vec::new();
    for r in roles.iter().take(50) {
        lines.push(format!(
            "- {} id={} color=#{:06X} pos={} mentionable={} perms={}",
            r.name,
            r.id,
            r.colors.primary_color & 0xFF_FF_FF,
            r.position,
            r.mentionable,
            r.permissions.bits()
        ));
    }
    let mut out = format!(
        "Roles in server {guild_id} ({} total):\n{}",
        roles.len(),
        lines.join("\n")
    );
    if roles.len() > 50 {
        out.push_str(&format!("\n... and {} more", roles.len() - 50));
    }
    Ok(truncate_output(out, 4000))
}

pub async fn search_members(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let query = get_str(args, "query", "");
    if query.is_empty() {
        return Err("search_members: 'query' is required.".into());
    }
    let limit = clamp_int_arg(args, "limit", 10, 1, 20);
    let members = discord
        .search_guild_members(guild_id, query)
        .limit(limit as u16)
        .await?
        .model()
        .await?;
    if members.is_empty() {
        return Ok(format!("No members found matching '{query}'."));
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
        format!("Members matching '{query}':\n{}", lines.join("\n")),
        4000,
    ))
}

pub async fn server_info(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let g = discord.guild(guild_id).await?.model().await?;
    Ok(format!(
        "Server '{}' (id={}): owner=<@{}>, members approx={}, explicit_content_filter={:?}, premium_tier={:?}, created via splash={}, description='{}'",
        g.name,
        g.id,
        g.owner_id,
        g.approximate_member_count.unwrap_or(0),
        g.explicit_content_filter,
        g.premium_tier,
        g.splash.is_some(),
        g.description.as_deref().unwrap_or("(none)")
    ))
}

pub async fn list_bans(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let limit = clamp_int_arg(args, "limit", 25, 1, 50) as usize;
    let bans = discord.bans(guild_id).await?.model().await?;
    if bans.is_empty() {
        return Ok("No banned users.".to_string());
    }
    let mut lines = Vec::new();
    for b in bans.iter().take(limit) {
        lines.push(format!(
            "- {} id={} reason='{}'",
            b.user.name,
            b.user.id,
            b.reason.as_deref().unwrap_or("(no reason)")
        ));
    }
    let mut out = format!("Banned users ({} total):\n{}", bans.len(), lines.join("\n"));
    if bans.len() > limit {
        out.push_str(&format!("\n... and {} more", bans.len() - limit));
    }
    Ok(truncate_output(out, 4000))
}

pub async fn get_messages(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("get_messages: {e}"))?;
    let count = clamp_int_arg(args, "count", 10, 1, 50) as u16;
    let messages = discord
        .channel_messages(channel_id)
        .limit(count)
        .await?
        .model()
        .await?;
    if messages.is_empty() {
        return Ok(format!("No recent messages in <#{channel_id}>."));
    }
    let mut lines = Vec::new();
    for m in messages.iter().take(count as usize) {
        let content = m
            .content
            .chars()
            .take(200)
            .collect::<String>()
            .replace('\n', " ");
        lines.push(format!(
            "- [{}] {}: {content} (msg_id={})",
            m.id, m.author.name, m.id
        ));
    }
    Ok(truncate_output(
        format!(
            "Last {} messages in <#{channel_id}>:\n{}",
            lines.len(),
            lines.join("\n")
        ),
        4000,
    ))
}

pub async fn send_message(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("send_message: {e}"))?;
    let content = get_str(args, "content", "");
    if content.is_empty() {
        return Err("send_message: 'content' is required and must not be empty.".into());
    }
    if content.chars().count() > 2000 {
        return Err("send_message: 'content' must be at most 2000 characters.".into());
    }
    let sent = discord
        .create_message(channel_id)
        .content(content)
        .await?
        .model()
        .await?;
    Ok(format!("Sent message {} to <#{}>", sent.id, channel_id))
}

pub async fn set_nickname(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("set_nickname: {e}"))?;
    let nickname = get_str(args, "nickname", "");
    let reason = get_str(args, "reason", "");
    let lowered = nickname.to_lowercase();
    let clear =
        nickname.is_empty() || lowered == "clear" || lowered == "reset" || lowered == "none";
    if !clear && (nickname.chars().count() > 32) {
        return Err("set_nickname: 'nickname' must be at most 32 characters.".into());
    }
    let request = if clear {
        discord.update_guild_member(guild_id, user_id).nick(None)
    } else {
        discord
            .update_guild_member(guild_id, user_id)
            .nick(Some(nickname))
    };
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }
    if clear {
        Ok(format!("Cleared nickname for user {user_id}"))
    } else {
        Ok(format!("Set nickname for user {user_id} to '{nickname}'"))
    }
}

pub async fn delete_role(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("delete_role: {e}"))?;
    discord.delete_role(guild_id, role_id).await?;
    Ok(format!("Deleted role {role_id}"))
}

pub async fn remove_timeout(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("remove_timeout: {e}"))?;
    let reason = get_str(args, "reason", "");
    let request = discord
        .update_guild_member(guild_id, user_id)
        .communication_disabled_until(None);
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }
    Ok(format!("Removed timeout for user {user_id}"))
}

pub async fn set_topic(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id = parse_channel_id(args, "channel_id").map_err(|e| format!("set_topic: {e}"))?;
    // `topic` may legitimately be empty (clear). Distinguish missing vs empty:
    // missing field -> error; present-but-empty -> clear.
    let has_topic = args.get("topic").is_some();
    if !has_topic {
        return Err("set_topic: 'topic' is required (pass empty string to clear).".into());
    }
    let topic = args["topic"].as_str().unwrap_or_default();
    if topic.chars().count() > 1024 {
        return Err("set_topic: 'topic' must be at most 1024 characters.".into());
    }
    discord.update_channel(channel_id).topic(topic).await?;
    if topic.is_empty() {
        Ok(format!("Cleared topic in <#{channel_id}>"))
    } else {
        Ok(format!("Set topic in <#{channel_id}> to '{topic}'"))
    }
}
