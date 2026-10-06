pub mod channels;
pub mod helpers;
pub mod moderation;
pub mod roles;
pub mod schemas;

use serde_json::Value;
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{marker::GuildMarker, Id};

pub use schemas::{build_tools_declaration, is_destructive, tool_names};

/// Central dispatcher: tool name (from Gemini `functionCall`) -> executor.
///
/// Each tool lives in its own module; this `match` is the only place that
/// needs to change when a new tool is added (Phase 3).
pub async fn execute_tool(
    name: &str,
    args: &Value,
    guild_id: Id<GuildMarker>,
    discord: &DiscordHttp,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match name {
        "create_role" => roles::execute(args, guild_id, discord).await,
        "rename_channel" => channels::execute(args, guild_id, discord).await,
        "purge_messages" => moderation::execute(args, guild_id, discord).await,
        _ => Err(format!("Unknown tool requested: {name}").into()),
    }
}
