use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::{
    channel::ChannelType,
    guild::Permissions,
    http::permission_overwrite::{PermissionOverwrite, PermissionOverwriteType},
    id::{
        marker::{ChannelMarker, GenericMarker, GuildMarker},
        Id,
    },
};

use super::helpers::{clamp_int_arg, get_str, parse_channel_id, parse_role_id, parse_user_id};

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

pub fn move_channel_schema() -> Value {
    json!({
        "name": "move_channel",
        "description": "Moves a channel into a category and/or reorders it. Pass a category channel ID as parent, or empty string to move out of categories. Position 0 = top.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to move." },
                "parent_id": { "type": "string", "description": "Category channel ID, or empty string for no category." },
                "position": { "type": "integer", "description": "New position 0-1000 (optional)." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn clone_channel_schema() -> Value {
    json!({
        "name": "clone_channel",
        "description": "Clones a channel: creates a new channel of the same kind with the same topic/NSFW/slowmode/parent. Give the new name.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to clone." },
                "new_name": { "type": "string", "description": "Name for the copy." }
            },
            "required": ["channel_id", "new_name"]
        }
    })
}

pub fn set_channel_nsfw_schema() -> Value {
    json!({
        "name": "set_channel_nsfw",
        "description": "Marks a text/announcement channel as NSFW (age-restricted) or removes the flag.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to change." },
                "nsfw": { "type": "boolean", "description": "true = age-restricted, false = not restricted." }
            },
            "required": ["channel_id", "nsfw"]
        }
    })
}

pub fn set_voice_limits_schema() -> Value {
    json!({
        "name": "set_voice_limits",
        "description": "Sets a voice/stage channel's bitrate (8000-384000) and user limit (0-99, 0 = unlimited). Omit a field to leave it unchanged.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Voice/stage channel." },
                "bitrate": { "type": "integer", "description": "Bitrate 8000-384000." },
                "user_limit": { "type": "integer", "description": "Max users 0-99 (0 = unlimited)." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn lock_channel_schema() -> Value {
    json!({
        "name": "lock_channel",
        "description": "Locks a text channel: denies Send Messages for @everyone so only staff can speak. Reversible with unlock_channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Text channel to lock." },
                "reason": { "type": "string", "description": "Optional audit-log reason." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn unlock_channel_schema() -> Value {
    json!({
        "name": "unlock_channel",
        "description": "Unlocks a channel locked by lock_channel: removes the @everyone send-deny overwrite.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to unlock." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn set_channel_permissions_schema() -> Value {
    json!({
        "name": "set_channel_permissions",
        "description": "Sets a channel permission overwrite for one role or member. Pass allow/deny as Discord permission bit integers (e.g. 1024 = View Channel, 2048 = Send Messages). Use list_roles to resolve IDs.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to edit." },
                "target_id": { "type": "string", "description": "Role ID (or <@&...>) or user ID the overwrite applies to." },
                "target_type": { "type": "string", "description": "Either 'role' or 'member'." },
                "allow": { "type": "integer", "description": "Permission bits to ALLOW (default 0)." },
                "deny": { "type": "integer", "description": "Permission bits to DENY (default 0)." }
            },
            "required": ["channel_id", "target_id", "target_type"]
        }
    })
}

pub fn clear_channel_permissions_schema() -> Value {
    json!({
        "name": "clear_channel_permissions",
        "description": "Deletes a channel permission overwrite for one role or member, resetting them to server defaults.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to edit." },
                "target_id": { "type": "string", "description": "Role or user ID whose overwrite is removed." },
                "target_type": { "type": "string", "description": "Either 'role' or 'member'." }
            },
            "required": ["channel_id", "target_id", "target_type"]
        }
    })
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

fn parse_optional_category(args: &Value) -> Result<Option<Id<ChannelMarker>>, String> {
    match args.get("parent_id") {
        None => Ok(None), // field absent = leave unchanged; handled by caller via flag
        Some(v) if v.is_null() => Ok(None),
        Some(v) => {
            let s = v.as_str().unwrap_or("").trim();
            if s.is_empty() || s.eq_ignore_ascii_case("none") || s == "0" {
                // Caller distinguishes "clear" by presence of the key; we encode
                // clear as Some(zero-check) — instead return a sentinel via Err?
                // Simpler: empty string means clear to top level.
                return Err("__CLEAR__".to_string());
            }
            let digits = s.trim_start_matches("<#").trim_end_matches('>').trim();
            digits
                .parse::<u64>()
                .map(Id::new)
                .map(Some)
                .map_err(|_| format!("Invalid parent category ID '{s}'."))
        }
    }
}

pub async fn move_channel(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("move_channel: {e}"))?;
    let has_parent = args.get("parent_id").is_some();
    let has_position = args.get("position").is_some();
    if !has_parent && !has_position {
        return Err("move_channel: provide 'parent_id' (category ID or empty to uncategorize) and/or 'position'.".into());
    }
    let mut req = discord.update_channel(channel_id);
    let mut desc = Vec::new();
    if has_parent {
        match parse_optional_category(args) {
            Ok(parent) => {
                req = req.parent_id(parent);
                desc.push(format!("category={}", parent.map(|p| p.to_string()).as_deref().unwrap_or("(top level)")));
            }
            Err(s) if s == "__CLEAR__" => {
                req = req.parent_id(None);
                desc.push("category=(top level)".to_string());
            }
            Err(e) => return Err(format!("move_channel: {e}").into()),
        }
    }
    if has_position {
        let pos = clamp_int_arg(args, "position", 0, 0, 1000);
        req = req.position(pos);
        desc.push(format!("position={pos}"));
    }
    req.await?;
    Ok(format!("Moved channel <#{channel_id}> ({})", desc.join(", ")))
}

pub async fn clone_channel(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("clone_channel: {e}"))?;
    let new_name = get_str(args, "new_name", "");
    if new_name.is_empty() {
        return Err("clone_channel: 'new_name' is required.".into());
    }
    if new_name.len() > 100 {
        return Err("clone_channel: 'new_name' must be at most 100 characters.".into());
    }
    let src = discord.channel(channel_id).await?.model().await?;
    let kind = match src.kind {
        ChannelType::GuildVoice => ChannelType::GuildVoice,
        ChannelType::GuildAnnouncement => ChannelType::GuildAnnouncement,
        ChannelType::GuildCategory => ChannelType::GuildCategory,
        _ => ChannelType::GuildText,
    };
    let mut req = discord.create_guild_channel(guild_id, new_name).kind(kind);
    if let Some(t) = src.topic.as_deref() {
        if !t.is_empty()
            && (kind == ChannelType::GuildText || kind == ChannelType::GuildAnnouncement)
        {
            req = req.topic(t);
        }
    }
    if src.nsfw.unwrap_or(false)
        && (kind == ChannelType::GuildText || kind == ChannelType::GuildAnnouncement)
    {
        req = req.nsfw(true);
    }
    if let Some(slow) = src.rate_limit_per_user {
        if slow > 0 {
            req = req.rate_limit_per_user(slow);
        }
    }
    if let Some(parent) = src.parent_id {
        req = req.parent_id(parent);
    }
    if let Some(bitrate) = src.bitrate {
        if kind == ChannelType::GuildVoice {
            req = req.bitrate(bitrate);
        }
    }
    if let Some(limit) = src.user_limit {
        if kind == ChannelType::GuildVoice {
            req = req.user_limit(limit.min(99) as u16);
        }
    }
    let created = req.await?.model().await?;
    Ok(format!(
        "Cloned <#{channel_id}> as '{}' (id={})",
        created.name.as_deref().unwrap_or(new_name),
        created.id
    ))
}

pub async fn set_channel_nsfw(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("set_channel_nsfw: {e}"))?;
    let nsfw = args
        .get("nsfw")
        .and_then(|v| v.as_bool())
        .ok_or("set_channel_nsfw: 'nsfw' must be true or false.")?;
    discord.update_channel(channel_id).nsfw(nsfw).await?;
    Ok(format!(
        "{} NSFW flag in <#{channel_id}>",
        if nsfw { "Enabled" } else { "Disabled" }
    ))
}

pub async fn set_voice_limits(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("set_voice_limits: {e}"))?;
    let has_bitrate = args.get("bitrate").is_some();
    let has_limit = args.get("user_limit").is_some();
    if !has_bitrate && !has_limit {
        return Err("set_voice_limits: provide 'bitrate' and/or 'user_limit'.".into());
    }
    let mut req = discord.update_channel(channel_id);
    if has_bitrate {
        let br = clamp_int_arg(args, "bitrate", 64000, 8000, 384000) as u32;
        req = req.bitrate(br);
    }
    if has_limit {
        let lim = clamp_int_arg(args, "user_limit", 0, 0, 99) as u16;
        req = req.user_limit(lim);
    }
    req.await?;
    Ok(format!("Updated voice limits in <#{channel_id}>"))
}

pub async fn lock_channel(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_http::request::AuditLogReason;
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("lock_channel: {e}"))?;
    let reason = get_str(args, "reason", "");
    let overwrite = PermissionOverwrite {
        allow: None,
        deny: Some(Permissions::SEND_MESSAGES),
        id: guild_id.cast(),
        kind: PermissionOverwriteType::Role,
    };
    let req = discord.update_channel_permission(channel_id, &overwrite);
    if reason.is_empty() {
        req.await?;
    } else {
        req.reason(reason).await?;
    }
    Ok(format!("Locked <#{channel_id}> (@everyone can no longer send messages)"))
}

pub async fn unlock_channel(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("unlock_channel: {e}"))?;
    discord
        .delete_channel_permission(channel_id)
        .role(guild_id.cast())
        .await?;
    Ok(format!("Unlocked <#{channel_id}> (@everyone overwrite removed)"))
}

fn parse_overwrite_target(args: &Value) -> Result<(Id<GenericMarker>, PermissionOverwriteType), String> {
    let kind_str = get_str(args, "target_type", "").to_lowercase();
    let kind = match kind_str.as_str() {
        "role" => PermissionOverwriteType::Role,
        "member" => PermissionOverwriteType::Member,
        _ => return Err("target_type must be 'role' or 'member'.".to_string()),
    };
    let raw = args.get("target_id").cloned().unwrap_or(Value::Null);
    let probe = serde_json::json!({ "target_id": raw });
    let id: Id<GenericMarker> = match kind {
        PermissionOverwriteType::Role => parse_role_id(&probe, "target_id")
            .map(|r| r.cast())
            .map_err(|e| e)?,
        PermissionOverwriteType::Member => parse_user_id(&probe, "target_id")
            .map(|u| u.cast())
            .map_err(|e| e)?,
        _ => return Err("target_type must be 'role' or 'member'.".to_string()),
    };
    Ok((id, kind))
}

pub async fn set_channel_permissions(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_http::request::AuditLogReason;
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("set_channel_permissions: {e}"))?;
    let (target, kind) =
        parse_overwrite_target(args).map_err(|e| format!("set_channel_permissions: {e}"))?;
    let allow_bits = args.get("allow").and_then(|v| v.as_u64()).unwrap_or(0);
    let deny_bits = args.get("deny").and_then(|v| v.as_u64()).unwrap_or(0);
    if allow_bits == 0 && deny_bits == 0 {
        return Err("set_channel_permissions: provide nonzero 'allow' and/or 'deny' bit integers.".into());
    }
    let overwrite = PermissionOverwrite {
        allow: Permissions::from_bits(allow_bits).map(Some).unwrap_or(None),
        deny: Permissions::from_bits(deny_bits).map(Some).unwrap_or(None),
        id: target,
        kind,
    };
    if overwrite.allow.is_none() && allow_bits != 0 {
        return Err("set_channel_permissions: 'allow' contains unknown permission bits.".into());
    }
    if overwrite.deny.is_none() && deny_bits != 0 {
        return Err("set_channel_permissions: 'deny' contains unknown permission bits.".into());
    }
    let reason = get_str(args, "reason", "");
    let req = discord.update_channel_permission(channel_id, &overwrite);
    if reason.is_empty() {
        req.await?;
    } else {
        req.reason(reason).await?;
    }
    Ok(format!(
        "Set overwrite in <#{channel_id}> for {kind:?} {target}: allow={allow_bits} deny={deny_bits}"
    ))
}

pub async fn clear_channel_permissions(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id = parse_channel_id(args, "channel_id")
        .map_err(|e| format!("clear_channel_permissions: {e}"))?;
    let kind_str = get_str(args, "target_type", "").to_lowercase();
    let probe = serde_json::json!({ "target_id": args.get("target_id").cloned().unwrap_or(Value::Null) });
    let req = discord.delete_channel_permission(channel_id);
    match kind_str.as_str() {
        "role" => {
            let role_id =
                parse_role_id(&probe, "target_id").map_err(|e| format!("clear_channel_permissions: {e}"))?;
            req.role(role_id).await?;
        }
        "member" => {
            let user_id =
                parse_user_id(&probe, "target_id").map_err(|e| format!("clear_channel_permissions: {e}"))?;
            req.member(user_id).await?;
        }
        _ => return Err("clear_channel_permissions: 'target_type' must be 'role' or 'member'.".into()),
    }
    Ok(format!("Cleared overwrite in <#{channel_id}>"))
}
