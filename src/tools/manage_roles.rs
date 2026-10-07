use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::{
    guild::{Permissions, RolePosition},
    id::{marker::GuildMarker, Id},
};

use super::helpers::{clamp_int_arg, get_str, parse_role_id, truncate_output};

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

pub fn role_info_schema() -> Value {
    json!({
        "name": "role_info",
        "description": "Shows one role's name, color, position, permissions bit integer, hoist, mentionable, managed flag. Check before editing/assigning.",
        "parameters": {
            "type": "object",
            "properties": {
                "role_id": { "type": "string", "description": "Role ID (or <@&...> mention)." }
            },
            "required": ["role_id"]
        }
    })
}

pub fn edit_role_schema() -> Value {
    json!({
        "name": "edit_role",
        "description": "Edits a role's name, color, hoist (show separately), and mentionable flag. Omit fields to leave unchanged.",
        "parameters": {
            "type": "object",
            "properties": {
                "role_id": { "type": "string", "description": "Role to edit." },
                "name": { "type": "string", "description": "New name (1-100 chars)." },
                "color": { "type": "integer", "description": "RGB decimal 0-16777215." },
                "hoist": { "type": "boolean", "description": "Show members separately in the sidebar." },
                "mentionable": { "type": "boolean", "description": "Whether anyone may @mention the role." },
                "reason": { "type": "string", "description": "Optional audit-log reason." }
            },
            "required": ["role_id"]
        }
    })
}

pub fn set_role_color_schema() -> Value {
    json!({
        "name": "set_role_color",
        "description": "Sets a role's sidebar color. Pass a decimal RGB integer (e.g. 16711680 = red) or 0 to clear.",
        "parameters": {
            "type": "object",
            "properties": {
                "role_id": { "type": "string", "description": "Role to recolor." },
                "color": { "type": "integer", "description": "RGB decimal 0-16777215." }
            },
            "required": ["role_id", "color"]
        }
    })
}

pub fn set_role_permissions_schema() -> Value {
    json!({
        "name": "set_role_permissions",
        "description": "Replaces a role's permission bit integer wholesale. DANGEROUS: list_roles/role_info first and only grant what the requester holds.",
        "parameters": {
            "type": "object",
            "properties": {
                "role_id": { "type": "string", "description": "Role to change." },
                "permissions": { "type": "integer", "description": "Full Discord permission bits for the role." },
                "reason": { "type": "string", "description": "Optional audit-log reason." }
            },
            "required": ["role_id", "permissions"]
        }
    })
}

pub fn set_role_position_schema() -> Value {
    json!({
        "name": "set_role_position",
        "description": "Moves a role to a new hierarchy position (higher number = higher). Discord re-sorts; the bot's own role must stay above it.",
        "parameters": {
            "type": "object",
            "properties": {
                "role_id": { "type": "string", "description": "Role to move." },
                "position": { "type": "integer", "description": "New position number (>=1)." }
            },
            "required": ["role_id", "position"]
        }
    })
}

pub fn list_role_members_schema() -> Value {
    json!({
        "name": "list_role_members",
        "description": "Shows how many members hold each role (role ID -> count). Use to answer 'who has role X' sizing or before deleting a role.",
        "parameters": { "type": "object", "properties": {}, "required": [] }
    })
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

pub async fn role_info(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("role_info: {e}"))?;
    let role = discord.role(guild_id, role_id).await?.model().await?;
    Ok(format!(
        "Role '{}' (id={}): color=#{:06X} pos={} perms={} hoist={} mentionable={} managed={}",
        role.name,
        role.id,
        role.colors.primary_color & 0xFF_FF_FF,
        role.position,
        role.permissions.bits(),
        role.hoist,
        role.mentionable,
        role.managed,
    ))
}

pub async fn edit_role(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_http::request::AuditLogReason;
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("edit_role: {e}"))?;
    if args.get("name").is_none()
        && args.get("color").is_none()
        && args.get("hoist").is_none()
        && args.get("mentionable").is_none()
    {
        return Err(
            "edit_role: provide at least one of 'name', 'color', 'hoist', 'mentionable'.".into(),
        );
    }
    let mut req = discord.update_role(guild_id, role_id);
    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        let name = name.trim();
        if name.is_empty() || name.len() > 100 {
            return Err("edit_role: 'name' must be 1-100 characters.".into());
        }
        req = req.name(Some(name));
    }
    if args.get("color").is_some() {
        let color = clamp_int_arg(args, "color", 0, 0, 0xFF_FF_FF) as u32;
        req = req.color(Some(color));
    }
    if let Some(hoist) = args.get("hoist").and_then(|v| v.as_bool()) {
        req = req.hoist(hoist);
    }
    if let Some(mentionable) = args.get("mentionable").and_then(|v| v.as_bool()) {
        req = req.mentionable(mentionable);
    }
    let reason = get_str(args, "reason", "");
    if reason.is_empty() {
        req.await?;
    } else {
        req.reason(reason).await?;
    }
    Ok(format!("Edited role {role_id}"))
}

pub async fn set_role_color(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("set_role_color: {e}"))?;
    let color = clamp_int_arg(args, "color", 0, 0, 0xFF_FF_FF) as u32;
    discord
        .update_role(guild_id, role_id)
        .color(Some(color))
        .await?;
    Ok(format!("Set role {role_id} color to #{color:06X}"))
}

pub async fn set_role_permissions(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use twilight_http::request::AuditLogReason;
    let role_id =
        parse_role_id(args, "role_id").map_err(|e| format!("set_role_permissions: {e}"))?;
    let bits = args
        .get("permissions")
        .and_then(|v| v.as_u64())
        .ok_or("set_role_permissions: 'permissions' bit integer is required.")?;
    let perms = Permissions::from_bits(bits)
        .ok_or("set_role_permissions: 'permissions' contains unknown permission bits.")?;
    let reason = get_str(args, "reason", "");
    let req = discord.update_role(guild_id, role_id).permissions(perms);
    if reason.is_empty() {
        req.await?;
    } else {
        req.reason(reason).await?;
    }
    Ok(format!("Set role {role_id} permissions to {bits}"))
}

pub async fn set_role_position(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("set_role_position: {e}"))?;
    let position = clamp_int_arg(args, "position", 1, 1, 250);
    // Fetch current roles so the position update sends the full ordering Discord expects.
    let mut roles = discord.roles(guild_id).await?.model().await?;
    let mut found = false;
    for r in roles.iter_mut() {
        if r.id == role_id {
            r.position = position as i64;
            found = true;
        }
    }
    if !found {
        return Err(
            format!("set_role_position: role {role_id} does not exist in this server.").into(),
        );
    }
    roles.sort_by_key(|r| r.position);
    let positions: Vec<RolePosition> = roles
        .iter()
        .map(|r| RolePosition {
            id: r.id,
            position: r.position.max(0) as u64,
        })
        .collect();
    discord.update_role_positions(guild_id, &positions).await?;
    Ok(format!("Moved role {role_id} to position {position}"))
}

pub async fn list_role_members(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let counts = discord.role_member_counts(guild_id).await?.model().await?;
    let roles = discord.roles(guild_id).await?.model().await?;
    let names: std::collections::HashMap<_, _> =
        roles.iter().map(|r| (r.id.get(), r.name.clone())).collect();
    let mut entries: Vec<(u64, u64)> = counts.into_iter().map(|(id, c)| (id.get(), c)).collect();
    entries.sort_by_key(|a| std::cmp::Reverse(a.1));
    if entries.is_empty() {
        return Ok("No role member counts returned.".to_string());
    }
    let mut lines = Vec::new();
    for (id, c) in entries.iter().take(50) {
        let name = names
            .get(id)
            .cloned()
            .unwrap_or_else(|| "(unknown)".to_string());
        lines.push(format!("- {name} id={id}: {c} members"));
    }
    Ok(truncate_output(
        format!("Role member counts:\n{}", lines.join("\n")),
        4000,
    ))
}
