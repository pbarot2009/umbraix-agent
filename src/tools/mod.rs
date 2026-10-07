pub mod channels;
pub mod extra;
pub mod guard;
pub mod helpers;
pub mod info;
pub mod moderation;
pub mod roles;
pub mod schemas;

use serde_json::Value;
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{marker::GuildMarker, Id};

pub use guard::ToolGuard;
pub use schemas::{build_tools_declaration, is_destructive, is_read_only, tool_names};

/// Central dispatcher: tool name (from Gemini `functionCall`) -> executor.
///
/// Each tool lives in its own module; this `match` is the only place that
/// needs a new arm when a tool is added. `tests/tools_schema.rs` verifies
/// every name in [`tool_names`] has a working arm here.
pub async fn execute_tool(
    name: &str,
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match name {
        // Read-only grounding.
        "list_channels" => info::list_channels(args, guild_id, discord).await,
        "user_info" => info::user_info(args, guild_id, discord).await,
        "list_roles" => extra::list_roles(args, guild_id, discord).await,
        "search_members" => extra::search_members(args, guild_id, discord).await,
        "server_info" => extra::server_info(args, guild_id, discord).await,
        "list_bans" => extra::list_bans(args, guild_id, discord).await,
        "get_messages" => extra::get_messages(args, guild_id, discord).await,
        // Messaging.
        "send_message" => extra::send_message(args, guild_id, discord).await,
        // Moderation.
        "kick_member" => moderation::kick(args, guild_id, discord).await,
        "ban_member" => moderation::ban(args, guild_id, discord).await,
        "unban_user" => moderation::unban(args, guild_id, discord).await,
        "timeout_member" => moderation::timeout(args, guild_id, discord).await,
        "remove_timeout" => extra::remove_timeout(args, guild_id, discord).await,
        "purge_messages" => moderation::purge(args, guild_id, discord).await,
        "set_nickname" => extra::set_nickname(args, guild_id, discord).await,
        // Channels.
        "create_channel" => channels::create(args, guild_id, discord).await,
        "rename_channel" => channels::rename(args, guild_id, discord).await,
        "delete_channel" => channels::delete(args, guild_id, discord).await,
        "set_slowmode" => channels::slowmode(args, guild_id, discord).await,
        "set_topic" => extra::set_topic(args, guild_id, discord).await,
        // Roles.
        "create_role" => roles::create(args, guild_id, discord).await,
        "assign_role" => roles::assign(args, guild_id, discord).await,
        "remove_role" => roles::remove(args, guild_id, discord).await,
        "delete_role" => extra::delete_role(args, guild_id, discord).await,
        _ => Err(format!("Unknown tool requested: {name}").into()),
    }
}
