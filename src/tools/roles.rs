use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{marker::GuildMarker, Id};

/// JSON schema advertised to Gemini for `create_role`.
pub fn schema() -> Value {
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

pub async fn execute(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let role_name = args["name"].as_str().unwrap_or("New Role").trim();
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
