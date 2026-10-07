use serde_json::{json, Value};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{marker::GuildMarker, Id};

use super::helpers::parse_user_id;

/// Read-only grounding tools. These never change server state; they exist
/// so the model can resolve real IDs before calling action tools instead
/// of inventing Snowflakes.
pub fn list_channels_schema() -> Value {
    json!({
        "name": "list_channels",
        "description": "Lists all channels in the server with their IDs and kinds. Call this first when a request names a channel but gives no ID.",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        }
    })
}

pub fn user_info_schema() -> Value {
    json!({
        "name": "user_info",
        "description": "Shows a member's display name, roles, and join date. Call this to confirm who a moderation or role action will affect.",
        "parameters": {
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Snowflake ID (or @mention) of the member to look up."
                }
            },
            "required": ["user_id"]
        }
    })
}

pub async fn list_channels(
    _args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let channels = discord.guild_channels(guild_id).await?.model().await?;

    if channels.is_empty() {
        return Ok("The server has no channels.".to_string());
    }

    fn kind_label(kind: twilight_model::channel::ChannelType) -> &'static str {
        use twilight_model::channel::ChannelType as T;
        match kind {
            T::GuildText => "text",
            T::GuildVoice => "voice",
            T::GuildAnnouncement => "announcement",
            T::GuildCategory => "category",
            T::GuildStageVoice => "stage",
            T::GuildForum => "forum",
            T::AnnouncementThread | T::PublicThread | T::PrivateThread => "thread",
            _ => "other",
        }
    }

    // Cap output so a huge server doesn't blow up the model's context.
    let mut lines = Vec::new();
    for channel in channels.iter().take(50) {
        let name = channel.name.as_deref().unwrap_or("(no name)");
        lines.push(format!(
            "- #{name} id={} kind={}",
            channel.id,
            kind_label(channel.kind)
        ));
    }
    let mut out = format!("Channels in server {guild_id}:\n{}", lines.join("\n"));
    if channels.len() > 50 {
        out.push_str(&format!("\n... and {} more", channels.len() - 50));
    }
    Ok(out)
}

pub async fn user_info(
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let user_id = parse_user_id(args, "user_id").map_err(|e| format!("user_info: {e}"))?;

    let member = discord
        .guild_member(guild_id, user_id)
        .await?
        .model()
        .await?;
    let roles = member
        .roles
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let joined = member
        .joined_at
        .map(|t| t.iso_8601().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let nick = member.nick.as_deref().unwrap_or("(no nickname)");
    let timed_out = member
        .communication_disabled_until
        .map(|t| t.iso_8601().to_string())
        .unwrap_or_else(|| "no".to_string());

    Ok(format!(
        "User {} (id={user_id}, nick='{nick}', bot: {}): {} role(s) [{}], joined {}, timed out until: {timed_out}",
        member.user.name,
        member.user.bot,
        member.roles.len(),
        if roles.is_empty() {
            "none".to_string()
        } else {
            roles
        },
        joined
    ))
}
