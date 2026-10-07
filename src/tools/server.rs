use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{
    marker::{ChannelMarker, EmojiMarker, GuildMarker, ScheduledEventMarker, WebhookMarker},
    Id,
};

use super::helpers::{clamp_int_arg, get_str, parse_channel_id, truncate_output};

fn parse_emoji_id(args: &Value, field: &str) -> Result<Id<EmojiMarker>, String> {
    if let Some(s) = args[field].as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Err(format!("Missing required field '{field}'."));
        }
        return s.parse::<u64>().map(Id::new).map_err(|_| format!("Invalid emoji ID '{s}'."));
    }
    if let Some(n) = args[field].as_u64() {
        if n == 0 {
            return Err(format!("Missing required field '{field}'."));
        }
        return Ok(Id::new(n));
    }
    Err(format!("Missing required field '{field}'."))
}

fn parse_webhook_id(args: &Value, field: &str) -> Result<Id<WebhookMarker>, String> {
    if let Some(s) = args[field].as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Err(format!("Missing required field '{field}'."));
        }
        return s.parse::<u64>().map(Id::new).map_err(|_| format!("Invalid webhook ID '{s}'."));
    }
    if let Some(n) = args[field].as_u64() {
        if n == 0 {
            return Err(format!("Missing required field '{field}'."));
        }
        return Ok(Id::new(n));
    }
    Err(format!("Missing required field '{field}'."))
}

fn parse_event_id(args: &Value, field: &str) -> Result<Id<ScheduledEventMarker>, String> {
    if let Some(s) = args[field].as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Err(format!("Missing required field '{field}'."));
        }
        return s.parse::<u64>().map(Id::new).map_err(|_| format!("Invalid event ID '{s}'."));
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

pub fn edit_server_schema() -> Value {
    json!({
        "name": "edit_server",
        "description": "Edits server settings: name, AFK channel/timeout, system channel, rules channel, verification level (0-4), content filter (0-2). Omit fields to leave unchanged.",
        "parameters": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "New server name (2-100 chars)." },
                "afk_channel_id": { "type": "string", "description": "Voice channel for AFK, or empty to clear." },
                "afk_timeout": { "type": "integer", "description": "AFK seconds: 60, 300, 900, 1800 or 3600." },
                "system_channel_id": { "type": "string", "description": "Channel for system messages, or empty to clear." },
                "rules_channel_id": { "type": "string", "description": "Rules channel ID (community servers)." },
                "verification_level": { "type": "integer", "description": "0=none 1=low 2=medium 3=high 4=very high." },
                "explicit_content_filter": { "type": "integer", "description": "0=disabled 1=members without roles 2=all members." }
            },
            "required": []
        }
    })
}

pub fn get_audit_logs_schema() -> Value {
    json!({
        "name": "get_audit_logs",
        "description": "Reads the server audit log: who kicked/banned/edited what and when. Use to investigate incidents or verify past actions.",
        "parameters": {
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Entries 1-100 (default 20)." }
            },
            "required": []
        }
    })
}

pub fn list_emojis_schema() -> Value {
    json!({
        "name": "list_emojis",
        "description": "Lists custom emojis with IDs, names, and animated flags. Resolve names to IDs before rename/delete.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn rename_emoji_schema() -> Value {
    json!({
        "name": "rename_emoji",
        "description": "Renames a custom emoji (2-32 chars, alphanumeric + underscores).",
        "parameters": {
            "type": "object",
            "properties": {
                "emoji_id": { "type": "string", "description": "Emoji to rename." },
                "name": { "type": "string", "description": "New name." }
            },
            "required": ["emoji_id", "name"]
        }
    })
}

pub fn delete_emoji_schema() -> Value {
    json!({
        "name": "delete_emoji",
        "description": "Permanently deletes a custom emoji from the server.",
        "parameters": {
            "type": "object",
            "properties": {
                "emoji_id": { "type": "string", "description": "Emoji to delete." }
            },
            "required": ["emoji_id"]
        }
    })
}

pub fn list_invites_schema() -> Value {
    json!({
        "name": "list_invites",
        "description": "Lists active server invites: codes, channels, inviter, uses/max uses, expiry. Use to audit or before deleting.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn list_channel_invites_schema() -> Value {
    json!({
        "name": "list_channel_invites",
        "description": "Lists invites created from one specific channel.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to list invites for." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn create_invite_schema() -> Value {
    json!({
        "name": "create_invite",
        "description": "Creates an invite link for a channel. Set max_age seconds (0 = never expires), max_uses (0 = unlimited), temporary membership.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel the invite points to." },
                "max_age": { "type": "integer", "description": "Seconds until expiry, 0-604800 (default 86400 = 1 day, 0 = never)." },
                "max_uses": { "type": "integer", "description": "Max uses 0-100 (default 0 = unlimited)." },
                "temporary": { "type": "boolean", "description": "Kick members on disconnect (default false)." }
            },
            "required": ["channel_id"]
        }
    })
}

pub fn delete_invite_schema() -> Value {
    json!({
        "name": "delete_invite",
        "description": "Revokes an invite by its code (the part after discord.gg/).",
        "parameters": {
            "type": "object",
            "properties": {
                "code": { "type": "string", "description": "Invite code, e.g. 'abc123'." }
            },
            "required": ["code"]
        }
    })
}

pub fn list_webhooks_schema() -> Value {
    json!({
        "name": "list_webhooks",
        "description": "Lists server webhooks with IDs, names, channels, and owners. Resolve before delete.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn create_webhook_schema() -> Value {
    json!({
        "name": "create_webhook",
        "description": "Creates a webhook in a text/announcement channel for feeds and integrations.",
        "parameters": {
            "type": "object",
            "properties": {
                "channel_id": { "type": "string", "description": "Channel to attach the webhook to." },
                "name": { "type": "string", "description": "Webhook name (1-80 chars)." }
            },
            "required": ["channel_id", "name"]
        }
    })
}

pub fn delete_webhook_schema() -> Value {
    json!({
        "name": "delete_webhook",
        "description": "Permanently deletes a webhook by ID.",
        "parameters": {
            "type": "object",
            "properties": {
                "webhook_id": { "type": "string", "description": "Webhook to delete." }
            },
            "required": ["webhook_id"]
        }
    })
}

pub fn list_scheduled_events_schema() -> Value {
    json!({
        "name": "list_scheduled_events",
        "description": "Lists scheduled events (name, IDs, start time, status, channel). Check before deleting.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn delete_scheduled_event_schema() -> Value {
    json!({
        "name": "delete_scheduled_event",
        "description": "Cancels/deletes a scheduled event by ID.",
        "parameters": {
            "type": "object",
            "properties": {
                "event_id": { "type": "string", "description": "Event to delete." }
            },
            "required": ["event_id"]
        }
    })
}

pub fn list_automod_rules_schema() -> Value {
    json!({
        "name": "list_automod_rules",
        "description": "Lists AutoMod rules: IDs, names, event types, enabled flags. Check before deleting.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

pub fn prune_preview_schema() -> Value {
    json!({
        "name": "prune_preview",
        "description": "Counts how many inactive members WOULD be kicked by a prune (days 1-30). Dry-run before prune_members.",
        "parameters": {
            "type": "object",
            "properties": {
                "days": { "type": "integer", "description": "Inactivity days 1-30 (default 7)." }
            },
            "required": []
        }
    })
}

pub fn prune_members_schema() -> Value {
    json!({
        "name": "prune_members",
        "description": "Kicks inactive members with no roles (days 1-30). IRREVERSIBLE — run prune_preview first and follow the confirmation protocol.",
        "parameters": {
            "type": "object",
            "properties": {
                "days": { "type": "integer", "description": "Inactivity days 1-30 (default 7)." },
                "reason": { "type": "string", "description": "Audit-log reason." }
            },
            "required": ["days"]
        }
    })
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

pub async fn edit_server(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_model::guild::{ExplicitContentFilter, VerificationLevel};
    if args.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        return Err("edit_server: provide at least one field to change.".into());
    }
    let mut req = discord.update_guild(guild_id);
    let mut changed = Vec::new();
    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        let name = name.trim();
        if name.chars().count() < 2 || name.chars().count() > 100 {
            return Err("edit_server: 'name' must be 2-100 characters.".into());
        }
        req = req.name(name);
        changed.push(format!("name='{name}'"));
    }
    if let Some(v) = args.get("afk_channel_id") {
        let s = v.as_str().unwrap_or("").trim();
        if s.is_empty() {
            req = req.afk_channel_id(None);
            changed.push("afk_channel=cleared".to_string());
        } else {
            let probe = json!({ "channel_id": v });
            let ch = super::helpers::parse_channel_id(&probe, "channel_id")
                .map_err(|e| format!("edit_server: bad afk_channel_id: {e}"))?;
            req = req.afk_channel_id(Some(ch));
            changed.push(format!("afk_channel=<#{ch}>"));
        }
    }
    if args.get("afk_timeout").is_some() {
        let t = clamp_int_arg(args, "afk_timeout", 300, 60, 3600);
        if ![60, 300, 900, 1800, 3600].contains(&t) {
            return Err("edit_server: 'afk_timeout' must be one of 60, 300, 900, 1800, 3600.".into());
        }
        req = req.afk_timeout(t);
        changed.push(format!("afk_timeout={t}s"));
    }
    if let Some(v) = args.get("system_channel_id") {
        let s = v.as_str().unwrap_or("").trim();
        if s.is_empty() {
            req = req.system_channel(None);
            changed.push("system_channel=cleared".to_string());
        } else {
            let probe = json!({ "channel_id": v });
            let ch = super::helpers::parse_channel_id(&probe, "channel_id")
                .map_err(|e| format!("edit_server: bad system_channel_id: {e}"))?;
            req = req.system_channel(Some(ch));
            changed.push(format!("system_channel=<#{ch}>"));
        }
    }
    if let Some(v) = args.get("rules_channel_id") {
        let probe = json!({ "channel_id": v });
        let ch = super::helpers::parse_channel_id(&probe, "channel_id")
            .map_err(|e| format!("edit_server: bad rules_channel_id: {e}"))?;
        req = req.rules_channel(Some(ch));
        changed.push(format!("rules_channel=<#{ch}>"));
    }
    if args.get("verification_level").is_some() {
        let lvl = clamp_int_arg(args, "verification_level", 0, 0, 4);
        let level = match lvl {
            0 => VerificationLevel::None,
            1 => VerificationLevel::Low,
            2 => VerificationLevel::Medium,
            3 => VerificationLevel::High,
            _ => VerificationLevel::VeryHigh,
        };
        req = req.verification_level(Some(level));
        changed.push(format!("verification={lvl}"));
    }
    if args.get("explicit_content_filter").is_some() {
        let f = clamp_int_arg(args, "explicit_content_filter", 0, 0, 2);
        let filter = match f {
            0 => ExplicitContentFilter::None,
            1 => ExplicitContentFilter::MembersWithoutRole,
            _ => ExplicitContentFilter::AllMembers,
        };
        req = req.explicit_content_filter(Some(filter));
        changed.push(format!("content_filter={f}"));
    }
    if changed.is_empty() {
        return Err("edit_server: no recognized fields to change.".into());
    }
    req.await?;
    Ok(format!("Updated server {guild_id}: {}", changed.join(", ")))
}

pub async fn get_audit_logs(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let limit = clamp_int_arg(args, "limit", 20, 1, 100) as u16;
    let log = discord.audit_log(guild_id).limit(limit).await?.model().await?;
    if log.entries.is_empty() {
        return Ok("Audit log is empty.".to_string());
    }
    let users: std::collections::HashMap<_, _> =
        log.users.iter().map(|u| (u.id.get(), u.name.clone())).collect();
    let mut lines = Vec::new();
    for e in log.entries.iter().take(limit as usize) {
        let who = e
            .user_id
            .and_then(|id| users.get(&id.get()).cloned())
            .unwrap_or_else(|| "(unknown)".to_string());
        lines.push(format!(
            "- {:?} by {who} target={:?} reason='{}'",
            e.action_type,
            e.target_id,
            e.reason.as_deref().unwrap_or("(none)")
        ));
    }
    Ok(truncate_output(
        format!("Last {} audit entries:\n{}", lines.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn list_emojis(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let emojis = discord.emojis(guild_id).await?.model().await?;
    if emojis.is_empty() {
        return Ok("No custom emojis.".to_string());
    }
    let mut lines = Vec::new();
    for e in emojis.iter().take(50) {
        lines.push(format!(
            "- :{}: id={} animated={} roles={}",
            e.name,
            e.id,
            e.animated,
            e.roles.len()
        ));
    }
    Ok(truncate_output(
        format!("Emojis ({}):\n{}", emojis.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn rename_emoji(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let emoji_id = parse_emoji_id(args, "emoji_id").map_err(|e| format!("rename_emoji: {e}"))?;
    let name = get_str(args, "name", "");
    if name.len() < 2 || name.len() > 32 {
        return Err("rename_emoji: 'name' must be 2-32 characters.".into());
    }
    discord.update_emoji(guild_id, emoji_id).name(name).await?;
    Ok(format!("Renamed emoji {emoji_id} to :{name}:"))
}

pub async fn delete_emoji(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let emoji_id = parse_emoji_id(args, "emoji_id").map_err(|e| format!("delete_emoji: {e}"))?;
    discord.delete_emoji(guild_id, emoji_id).await?;
    Ok(format!("Deleted emoji {emoji_id}"))
}

fn fmt_invite(code: &str, channel: String, inviter: String, uses: String) -> String {
    format!("- discord.gg/{code} channel={channel} by {inviter} {uses}")
}

pub async fn list_invites(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let invites = discord.guild_invites(guild_id).await?.model().await?;
    if invites.is_empty() {
        return Ok("No active invites.".to_string());
    }
    let mut lines = Vec::new();
    for i in invites.iter().take(50) {
        lines.push(fmt_invite(
            &i.code,
            i.channel.as_ref().map(|c| format!("#{}", c.name.as_deref().unwrap_or("?"))).unwrap_or("(?)".to_string()),
            i.inviter.as_ref().map(|u| u.name.clone()).unwrap_or("(?)".to_string()),
            format!("uses={:?}/{:?}", i.uses, i.max_uses),
        ));
    }
    Ok(truncate_output(
        format!("Invites ({}):\n{}", invites.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn list_channel_invites(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id =
        parse_channel_id(args, "channel_id").map_err(|e| format!("list_channel_invites: {e}"))?;
    let invites = discord.channel_invites(channel_id).await?.model().await?;
    if invites.is_empty() {
        return Ok(format!("No invites for <#{channel_id}>."));
    }
    let mut lines = Vec::new();
    for i in invites.iter().take(50) {
        lines.push(fmt_invite(
            &i.code,
            format!("<#{channel_id}>"),
            i.inviter.as_ref().map(|u| u.name.clone()).unwrap_or("(?)".to_string()),
            format!("uses={:?}/{:?}", i.uses, i.max_uses),
        ));
    }
    Ok(truncate_output(
        format!("Invites for <#{channel_id}>:\n{}", lines.join("\n")),
        4000,
    ))
}

pub async fn create_invite(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id: Id<ChannelMarker> =
        parse_channel_id(args, "channel_id").map_err(|e| format!("create_invite: {e}"))?;
    let max_age = clamp_int_arg(args, "max_age", 86400, 0, 604800) as u32;
    let max_uses = clamp_int_arg(args, "max_uses", 0, 0, 100) as u16;
    let temporary = args.get("temporary").and_then(|v| v.as_bool()).unwrap_or(false);
    let invite = discord
        .create_invite(channel_id)
        .max_age(max_age)
        .max_uses(max_uses)
        .temporary(temporary)
        .await?
        .model()
        .await?;
    Ok(format!(
        "Created invite https://discord.gg/{} for <#{channel_id}> (expires in {}s, max uses {})",
        invite.code, max_age, max_uses
    ))
}

pub async fn delete_invite(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let code = get_str(args, "code", "");
    if code.is_empty() {
        return Err("delete_invite: 'code' is required (the part after discord.gg/).".into());
    }
    let code = code.trim().trim_start_matches("https://discord.gg/").trim_start_matches("discord.gg/");
    discord.delete_invite(code).await?;
    Ok(format!("Revoked invite discord.gg/{code}"))
}

pub async fn list_webhooks(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let hooks = discord.guild_webhooks(guild_id).await?.model().await?;
    if hooks.is_empty() {
        return Ok("No webhooks.".to_string());
    }
    let mut lines = Vec::new();
    for w in hooks.iter().take(50) {
        lines.push(format!(
            "- {} id={} channel={:?} by {}",
            w.name.as_deref().unwrap_or("(no name)"),
            w.id,
            w.channel_id,
            w.user.as_ref().map(|u| u.name.clone()).unwrap_or("(?)".to_string()),
        ));
    }
    Ok(truncate_output(
        format!("Webhooks ({}):\n{}", hooks.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn create_webhook(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channel_id: Id<ChannelMarker> =
        parse_channel_id(args, "channel_id").map_err(|e| format!("create_webhook: {e}"))?;
    let name = get_str(args, "name", "");
    if name.is_empty() || name.len() > 80 {
        return Err("create_webhook: 'name' must be 1-80 characters.".into());
    }
    let hook = discord.create_webhook(channel_id, name).await?.model().await?;
    Ok(format!(
        "Created webhook '{}' (id={}) in <#{channel_id}>",
        hook.name.as_deref().unwrap_or(name),
        hook.id
    ))
}

pub async fn delete_webhook(
    args: &Value,
    _guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let webhook_id =
        parse_webhook_id(args, "webhook_id").map_err(|e| format!("delete_webhook: {e}"))?;
    discord.delete_webhook(webhook_id).await?;
    Ok(format!("Deleted webhook {webhook_id}"))
}

pub async fn list_scheduled_events(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let events = discord.guild_scheduled_events(guild_id).await?.model().await?;
    if events.is_empty() {
        return Ok("No scheduled events.".to_string());
    }
    let mut lines = Vec::new();
    for e in events.iter().take(25) {
        lines.push(format!(
            "- {} id={} status={:?} channel={:?}",
            e.name, e.id, e.status, e.channel_id
        ));
    }
    Ok(truncate_output(
        format!("Scheduled events ({}):\n{}", events.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn delete_scheduled_event(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let event_id =
        parse_event_id(args, "event_id").map_err(|e| format!("delete_scheduled_event: {e}"))?;
    discord.delete_guild_scheduled_event(guild_id, event_id).await?;
    Ok(format!("Deleted scheduled event {event_id}"))
}

pub async fn list_automod_rules(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let rules = discord.auto_moderation_rules(guild_id).await?.model().await?;
    if rules.is_empty() {
        return Ok("No AutoMod rules.".to_string());
    }
    let mut lines = Vec::new();
    for r in rules.iter().take(25) {
        lines.push(format!(
            "- {} id={} event={:?} enabled={}",
            r.name, r.id, r.event_type, r.enabled
        ));
    }
    Ok(truncate_output(
        format!("AutoMod rules ({}):\n{}", rules.len(), lines.join("\n")),
        4000,
    ))
}

pub async fn prune_preview(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let days = clamp_int_arg(args, "days", 7, 1, 30) as u16;
    let count = discord.guild_prune_count(guild_id).days(days).await?.model().await?;
    Ok(format!(
        "Prune preview: {} members would be kicked for {days}d inactivity (use prune_members to execute).",
        count.pruned
    ))
}

pub async fn prune_members(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_http::request::AuditLogReason;
    let days = clamp_int_arg(args, "days", 7, 1, 30) as u16;
    let reason = get_str(args, "reason", "");
    let req = discord.create_guild_prune(guild_id).days(days).compute_prune_count(true);
    let res = if reason.is_empty() {
        req.await?
    } else {
        req.reason(reason).await?
    };
    let body = res.model().await?;
    Ok(format!(
        "Pruned {} members for {days}d inactivity.",
        body.pruned
    ))
}
