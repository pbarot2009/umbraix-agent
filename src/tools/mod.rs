pub mod channels;
pub mod extra;
pub mod guard;
pub mod helpers;
pub mod info;
pub mod manage_channels;
pub mod manage_roles;
pub mod messages;
pub mod moderation;
pub mod roles;
pub mod schemas;
pub mod server;
pub mod threads;
pub mod voice;

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
        "channel_details" => messages::channel_details(args, guild_id, discord).await,
        "list_active_threads" => threads::list_active_threads(args, guild_id, discord).await,
        "user_info" => info::user_info(args, guild_id, discord).await,
        "list_roles" => extra::list_roles(args, guild_id, discord).await,
        "role_info" => manage_roles::role_info(args, guild_id, discord).await,
        "list_role_members" => manage_roles::list_role_members(args, guild_id, discord).await,
        "search_members" => extra::search_members(args, guild_id, discord).await,
        "list_members" => voice::list_members(args, guild_id, discord).await,
        "server_info" => extra::server_info(args, guild_id, discord).await,
        "list_bans" => extra::list_bans(args, guild_id, discord).await,
        "get_messages" => extra::get_messages(args, guild_id, discord).await,
        "get_message_details" => messages::get_message_details(args, guild_id, discord).await,
        "get_audit_logs" => server::get_audit_logs(args, guild_id, discord).await,
        "list_emojis" => server::list_emojis(args, guild_id, discord).await,
        "list_invites" => server::list_invites(args, guild_id, discord).await,
        "list_channel_invites" => server::list_channel_invites(args, guild_id, discord).await,
        "list_pins" => messages::list_pins(args, guild_id, discord).await,
        "list_webhooks" => server::list_webhooks(args, guild_id, discord).await,
        "list_scheduled_events" => server::list_scheduled_events(args, guild_id, discord).await,
        "list_automod_rules" => server::list_automod_rules(args, guild_id, discord).await,
        "list_thread_members" => threads::list_thread_members(args, guild_id, discord).await,
        "get_voice_state" => voice::get_voice_state(args, guild_id, discord).await,
        "prune_preview" => server::prune_preview(args, guild_id, discord).await,
        // Messaging.
        "send_message" => extra::send_message(args, guild_id, discord).await,
        "edit_message" => messages::edit_message(args, guild_id, discord).await,
        "delete_single_message" => messages::delete_single_message(args, guild_id, discord).await,
        "pin_message" => messages::pin_message(args, guild_id, discord).await,
        "unpin_message" => messages::unpin_message(args, guild_id, discord).await,
        "publish_message" => messages::publish_message(args, guild_id, discord).await,
        // Moderation.
        "kick_member" => moderation::kick(args, guild_id, discord).await,
        "ban_member" => moderation::ban(args, guild_id, discord).await,
        "unban_user" => moderation::unban(args, guild_id, discord).await,
        "timeout_member" => moderation::timeout(args, guild_id, discord).await,
        "remove_timeout" => extra::remove_timeout(args, guild_id, discord).await,
        "purge_messages" => moderation::purge(args, guild_id, discord).await,
        "set_nickname" => extra::set_nickname(args, guild_id, discord).await,
        "warn_member" => voice::warn_member(args, guild_id, discord).await,
        "prune_members" => server::prune_members(args, guild_id, discord).await,
        // Channels.
        "create_channel" => channels::create(args, guild_id, discord).await,
        "rename_channel" => channels::rename(args, guild_id, discord).await,
        "delete_channel" => channels::delete(args, guild_id, discord).await,
        "move_channel" => manage_channels::move_channel(args, guild_id, discord).await,
        "clone_channel" => manage_channels::clone_channel(args, guild_id, discord).await,
        "set_slowmode" => channels::slowmode(args, guild_id, discord).await,
        "set_topic" => extra::set_topic(args, guild_id, discord).await,
        "set_channel_nsfw" => manage_channels::set_channel_nsfw(args, guild_id, discord).await,
        "set_voice_limits" => manage_channels::set_voice_limits(args, guild_id, discord).await,
        "lock_channel" => manage_channels::lock_channel(args, guild_id, discord).await,
        "unlock_channel" => manage_channels::unlock_channel(args, guild_id, discord).await,
        "set_channel_permissions" => {
            manage_channels::set_channel_permissions(args, guild_id, discord).await
        }
        "clear_channel_permissions" => {
            manage_channels::clear_channel_permissions(args, guild_id, discord).await
        }
        // Roles.
        "create_role" => roles::create(args, guild_id, discord).await,
        "edit_role" => manage_roles::edit_role(args, guild_id, discord).await,
        "set_role_color" => manage_roles::set_role_color(args, guild_id, discord).await,
        "set_role_permissions" => manage_roles::set_role_permissions(args, guild_id, discord).await,
        "set_role_position" => manage_roles::set_role_position(args, guild_id, discord).await,
        "assign_role" => roles::assign(args, guild_id, discord).await,
        "remove_role" => roles::remove(args, guild_id, discord).await,
        "delete_role" => extra::delete_role(args, guild_id, discord).await,
        // Voice.
        "move_voice_member" => voice::move_voice_member(args, guild_id, discord).await,
        "disconnect_voice_member" => voice::disconnect_voice_member(args, guild_id, discord).await,
        "mute_voice_member" => voice::mute_voice_member(args, guild_id, discord).await,
        "unmute_voice_member" => voice::unmute_voice_member(args, guild_id, discord).await,
        "deafen_voice_member" => voice::deafen_voice_member(args, guild_id, discord).await,
        "undeafen_voice_member" => voice::undeafen_voice_member(args, guild_id, discord).await,
        // Threads.
        "create_thread" => threads::create_thread(args, guild_id, discord).await,
        "thread_from_message" => threads::thread_from_message(args, guild_id, discord).await,
        "join_thread" => threads::join_thread(args, guild_id, discord).await,
        "archive_thread" => threads::archive_thread(args, guild_id, discord).await,
        "add_thread_member" => threads::add_thread_member(args, guild_id, discord).await,
        "remove_thread_member" => threads::remove_thread_member(args, guild_id, discord).await,
        // Invites / webhooks / emojis / events / server.
        "create_invite" => server::create_invite(args, guild_id, discord).await,
        "delete_invite" => server::delete_invite(args, guild_id, discord).await,
        "create_webhook" => server::create_webhook(args, guild_id, discord).await,
        "delete_webhook" => server::delete_webhook(args, guild_id, discord).await,
        "rename_emoji" => server::rename_emoji(args, guild_id, discord).await,
        "delete_emoji" => server::delete_emoji(args, guild_id, discord).await,
        "delete_scheduled_event" => server::delete_scheduled_event(args, guild_id, discord).await,
        "edit_server" => server::edit_server(args, guild_id, discord).await,
        _ => Err(format!("Unknown tool requested: {name}").into()),
    }
}
