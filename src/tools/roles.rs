use serde_json::{json, Value};
use twilight_http::{request::AuditLogReason, Client as DiscordHttp};
use twilight_model::id::{marker::GuildMarker, Id};

use super::helpers::{get_str, parse_role_id, parse_user_id};

/// JSON schema advertised to Gemini for `create_role`.
pub fn create_schema() -> Value {
    json!({
        "name": "create_role",
        "description": "Creates a new role in the guild with an optional RGB color code.",
        "parameters": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the role (e.g. 'Causiy game dev')"
                },
                "color": {
                    "type": "integer",
                    "description": "Optional RGB color in decimal (e.g., 5793266 for blurple, 16711680 for red)"
                }
            },
            "required": ["name"]
        }
    })
}

pub fn assign_schema() -> Value {
    json!({
        "name": "assign_role",
        "description": "Gives a role to a server member.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID (or @mention) of the member."
                },
                "role_id": {
                    "type": "string",
                    "description": "Snowflake ID (or <@&...> mention) of the role to give."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional audit-log reason shown to moderators."
                }
            },
            "required": ["user_id", "role_id"]
        }
    })
}

pub fn remove_schema() -> Value {
    json!({
        "name": "remove_role",
        "description": "Takes a role away from a server member.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID (or @mention) of the member."
                },
                "role_id": {
                    "type": "string",
                    "description": "Snowflake ID (or <@&...> mention) of the role to remove."
                },
                "reason": {
                    "type": "string",
                    "description": "Optional audit-log reason shown to moderators."
                }
            },
            "required": ["user_id", "role_id"]
        }
    })
}

pub async fn create(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let role_name = get_str(args, "name", "New Role");
    let role_name = if role_name.is_empty() {
        "New Role"
    } else {
        role_name
    };

    let mut builder = discord.create_role(guild_id).name(role_name);

    if let Some(color) = args["color"].as_u64() {
        builder = builder.color(color as u32);
    }

    let role = builder.await?.model().await?;
    Ok(format!(
        "Successfully created role '{}' with ID {}",
        role.name, role.id
    ))
}

pub async fn assign(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("assign_role: {e}"))?;
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("assign_role: {e}"))?;
    let reason = get_str(args, "reason", "");

    let request = discord.add_guild_member_role(guild_id, user_id, role_id);
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }

    Ok(format!(
        "Successfully gave role {role_id} to user {user_id}"
    ))
}

pub async fn remove(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("remove_role: {e}"))?;
    let role_id = parse_role_id(args, "role_id").map_err(|e| format!("remove_role: {e}"))?;
    let reason = get_str(args, "reason", "");

    let request = discord.remove_guild_member_role(guild_id, user_id, role_id);
    if reason.is_empty() {
        request.await?;
    } else {
        request.reason(reason).await?;
    }

    Ok(format!(
        "Successfully removed role {role_id} from user {user_id}"
    ))
}
