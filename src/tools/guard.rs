use serde_json::Value;
use std::sync::Arc;
use twilight_model::{
    guild::Permissions,
    id::{
        marker::{ChannelMarker, GuildMarker, RoleMarker, UserMarker},
        Id,
    },
};

use super::helpers::{get_str, parse_channel_id, parse_role_id, parse_user_id};
use crate::{
    access::{Authority, GuildSnapshot},
    context::Context,
};

/// Enforces the *requester's* Discord permissions on every tool call.
///
/// Without this, anyone with a BYOK key could make the bot ban people
/// using the bot's own permissions. Checks, in order:
/// 1. every referenced channel/role belongs to this guild (no cross-server actions)
/// 2. the requester holds the Discord permission the action needs
///    (channel overwrites honored when the channel is cached)
/// 3. role hierarchy: targets and roles must sit below the requester's top role
/// 4. no granting roles with permissions the requester lacks; no mass mentions without permission
pub struct ToolGuard {
    pub guild_id: Id<GuildMarker>,
    pub snapshot: Arc<GuildSnapshot>,
    pub authority: Authority,
    /// The bot's own user id, when known. Used to block self-harm
    /// (the model must never kick/ban/timeout the bot itself).
    pub bot_id: Option<Id<UserMarker>>,
}

pub fn required_permission(tool: &str) -> Option<Permissions> {
    Some(match tool {
        "list_bans" => Permissions::BAN_MEMBERS,
        "get_messages" | "get_message_details" | "list_pins" => {
            Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY
        }
        "channel_details" | "list_active_threads" | "list_thread_members" => {
            Permissions::VIEW_CHANNEL
        }
        "list_channel_invites" => Permissions::MANAGE_CHANNELS | Permissions::VIEW_CHANNEL,
        "send_message" | "publish_message" => {
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES
        }
        "edit_message" | "delete_single_message" => {
            Permissions::VIEW_CHANNEL | Permissions::MANAGE_MESSAGES
        }
        "pin_message" | "unpin_message" => Permissions::VIEW_CHANNEL | Permissions::MANAGE_MESSAGES,
        "kick_member" => Permissions::KICK_MEMBERS,
        "ban_member" | "unban_user" => Permissions::BAN_MEMBERS,
        "timeout_member" | "remove_timeout" => Permissions::MODERATE_MEMBERS,
        "purge_messages" => Permissions::MANAGE_MESSAGES,
        "set_nickname" => Permissions::MANAGE_NICKNAMES,
        "warn_member" => Permissions::MODERATE_MEMBERS,
        "prune_members" | "prune_preview" => Permissions::KICK_MEMBERS,
        "create_channel"
        | "rename_channel"
        | "delete_channel"
        | "set_slowmode"
        | "set_topic"
        | "move_channel"
        | "clone_channel"
        | "set_channel_nsfw"
        | "set_voice_limits"
        | "lock_channel"
        | "unlock_channel"
        | "set_channel_permissions"
        | "clear_channel_permissions" => Permissions::MANAGE_CHANNELS,
        "create_role"
        | "assign_role"
        | "remove_role"
        | "delete_role"
        | "edit_role"
        | "set_role_color"
        | "set_role_permissions"
        | "set_role_position" => Permissions::MANAGE_ROLES,
        "list_role_members" | "role_info" => Permissions::MANAGE_ROLES,
        "move_voice_member" | "disconnect_voice_member" => Permissions::MOVE_MEMBERS,
        "mute_voice_member" | "unmute_voice_member" => Permissions::MUTE_MEMBERS,
        "deafen_voice_member" | "undeafen_voice_member" => Permissions::DEAFEN_MEMBERS,
        "get_voice_state" => Permissions::VIEW_CHANNEL,
        "create_thread"
        | "thread_from_message"
        | "join_thread"
        | "archive_thread"
        | "add_thread_member"
        | "remove_thread_member" => Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
        "create_invite" | "delete_invite" => Permissions::CREATE_INVITE,
        "list_webhooks" | "create_webhook" | "delete_webhook" => Permissions::MANAGE_WEBHOOKS,
        "list_emojis" | "rename_emoji" | "delete_emoji" => Permissions::MANAGE_GUILD_EXPRESSIONS,
        "list_scheduled_events" | "delete_scheduled_event" => Permissions::MANAGE_EVENTS,
        "list_automod_rules" => Permissions::MANAGE_GUILD,
        "get_audit_logs" => Permissions::VIEW_AUDIT_LOG,
        "edit_server" => Permissions::MANAGE_GUILD,
        _ => return None,
    })
}

fn targets_member(tool: &str) -> bool {
    matches!(
        tool,
        "kick_member"
            | "ban_member"
            | "timeout_member"
            | "remove_timeout"
            | "set_nickname"
            | "assign_role"
            | "remove_role"
            | "warn_member"
            | "move_voice_member"
            | "disconnect_voice_member"
            | "mute_voice_member"
            | "unmute_voice_member"
            | "deafen_voice_member"
            | "undeafen_voice_member"
            | "add_thread_member"
            | "remove_thread_member"
    )
}

fn targets_role(tool: &str) -> bool {
    matches!(
        tool,
        "assign_role"
            | "remove_role"
            | "delete_role"
            | "edit_role"
            | "set_role_color"
            | "set_role_permissions"
            | "set_role_position"
    )
}

pub fn permission_label(p: Permissions) -> String {
    const NAMES: &[(Permissions, &str)] = &[
        (Permissions::ADMINISTRATOR, "Administrator"),
        (Permissions::MANAGE_GUILD, "Manage Server"),
        (Permissions::MANAGE_CHANNELS, "Manage Channels"),
        (Permissions::MANAGE_ROLES, "Manage Roles"),
        (
            Permissions::MANAGE_GUILD_EXPRESSIONS,
            "Manage Emojis/Stickers",
        ),
        (Permissions::MANAGE_WEBHOOKS, "Manage Webhooks"),
        (Permissions::MANAGE_EVENTS, "Manage Events"),
        (Permissions::CREATE_EVENTS, "Create Events"),
        (Permissions::CREATE_INVITE, "Create Invite"),
        (Permissions::BAN_MEMBERS, "Ban Members"),
        (Permissions::KICK_MEMBERS, "Kick Members"),
        (Permissions::MODERATE_MEMBERS, "Timeout Members"),
        (Permissions::MANAGE_MESSAGES, "Manage Messages"),
        (Permissions::MANAGE_NICKNAMES, "Manage Nicknames"),
        (Permissions::MANAGE_THREADS, "Manage Threads"),
        (Permissions::MOVE_MEMBERS, "Move Members"),
        (Permissions::MUTE_MEMBERS, "Mute Members"),
        (Permissions::DEAFEN_MEMBERS, "Deafen Members"),
        (Permissions::PRIORITY_SPEAKER, "Priority Speaker"),
        (Permissions::VIEW_AUDIT_LOG, "View Audit Log"),
        (Permissions::VIEW_CHANNEL, "View Channel"),
        (Permissions::SEND_MESSAGES, "Send Messages"),
        (
            Permissions::SEND_MESSAGES_IN_THREADS,
            "Send Messages in Threads",
        ),
        (Permissions::READ_MESSAGE_HISTORY, "Read Message History"),
        (Permissions::ADD_REACTIONS, "Add Reactions"),
        (Permissions::USE_EXTERNAL_EMOJIS, "Use External Emojis"),
        (Permissions::CONNECT, "Connect Voice"),
        (Permissions::SPEAK, "Speak in Voice"),
        (Permissions::MENTION_EVERYONE, "Mention @everyone"),
        (Permissions::CHANGE_NICKNAME, "Change Nickname"),
    ];
    let names: Vec<&str> = NAMES
        .iter()
        .filter(|(flag, _)| p.contains(*flag))
        .map(|(_, n)| *n)
        .collect();
    if names.is_empty() {
        format!("{p:?}")
    } else {
        names.join(" + ")
    }
}

fn deny(msg: impl Into<String>) -> Result<(), String> {
    Err(format!("Permission denied: {}", msg.into()))
}

impl ToolGuard {
    pub async fn check(&self, ctx: &Context, tool: &str, args: &Value) -> Result<(), String> {
        let channel = if args.get("channel_id").is_some() {
            parse_channel_id(args, "channel_id").ok()
        } else {
            None
        };
        if let Some(ch) = channel {
            self.ensure_channel_in_guild(ctx, ch).await?;
        }

        let auth = &self.authority;
        let mut required = required_permission(tool);
        if tool == "set_nickname" {
            if let Ok(target) = parse_user_id(args, "user_id") {
                if target == auth.user_id {
                    required = Some(Permissions::CHANGE_NICKNAME);
                }
            }
        }

        if let Some(req) = required {
            if !auth.has(req) {
                let missing = req - (req & auth.permissions);
                return deny(format!(
                    "the requester lacks **{}** in this server. Ask a server admin/owner to grant it, or ask them to run this action. Do not retry without it.",
                    permission_label(missing)
                ));
            }
            if let Some(ch) = channel {
                // Administrator bypasses channel overwrites entirely (Discord
                // semantics), as do the guild owner and bot owner. Everyone
                // else is checked against the cached overwrite view.
                let admin_bypass = auth.permissions.contains(Permissions::ADMINISTRATOR);
                if !auth.bypass && !auth.is_guild_owner && !admin_bypass {
                    // Channel overwrites are only checked when cached. On a
                    // cache miss we deliberately fail OPEN (rely on the
                    // guild-level check above) rather than false-denying —
                    // Discord itself will still block the bot if *it* lacks
                    // access, and that error is surfaced with a clear hint.
                    if let Ok(cp) = ctx.cache.permissions().in_channel(auth.user_id, ch) {
                        if !cp.contains(req) {
                            return deny(format!(
                                "the requester lacks **{}** in <#{ch}> (channel overwrite). Ask a moderator with Manage Channels to grant it there.",
                                permission_label(req - (req & cp))
                            ));
                        }
                    }
                }
            }
        }

        if tool == "send_message" {
            let content = get_str(args, "content", "");
            if (content.contains("@everyone") || content.contains("@here"))
                && !auth.has(Permissions::MENTION_EVERYONE)
            {
                return deny("the requester can't mention @everyone/@here (needs Mention Everyone). Remove the mass mention or ask a moderator.");
            }
        }

        if targets_role(tool) {
            let role_id = parse_role_id(args, "role_id").map_err(|e| format!("{tool}: {e}"))?;
            self.check_role(ctx, tool, role_id).await?;
            // set_role_permissions replaces the whole bitset: the requester may
            // not grant permissions they don't hold themselves.
            if tool == "set_role_permissions" && !auth.bypass && !auth.is_guild_owner {
                if let Some(bits) = super::helpers::parse_permission_bits(args, "permissions") {
                    if let Some(wanted) = Permissions::from_bits(bits) {
                        if !auth.permissions.contains(wanted) {
                            return deny(format!(
                                "the requested permission set ({bits}) includes permissions the requester doesn't have. They can't grant what they don't hold — ask an admin who has those permissions."
                            ));
                        }
                    }
                }
            }
        }

        // Self/bot protection runs even for the bot owner bypass: the model
        // must never kick/ban/timeout itself or the bot.
        if let Ok(target) = parse_user_id(args, "user_id") {
            let bot_id_u64: Option<u64> = self
                .bot_id
                .map(|b| b.get())
                .or_else(|| ctx.bot_user_id.get().map(|b| b.get()));
            if let Some(bot_u64) = bot_id_u64 {
                if target.get() == bot_u64 {
                    return Err(format!(
                        "{tool}: refusing to target the bot itself. Ask for clarification instead."
                    ));
                }
            }
            if matches!(
                tool,
                "kick_member"
                    | "ban_member"
                    | "timeout_member"
                    | "mute_voice_member"
                    | "deafen_voice_member"
                    | "disconnect_voice_member"
            ) && target == auth.user_id
            {
                return Err(format!(
                    "{tool}: refusing to moderate the requester themselves. Ask for clarification instead."
                ));
            }
        }

        if targets_member(tool) && !auth.bypass {
            let target = parse_user_id(args, "user_id").map_err(|e| format!("{tool}: {e}"))?;
            self.check_member_target(ctx, tool, target).await?;
        }
        Ok(())
    }

    async fn check_role(
        &self,
        ctx: &Context,
        tool: &str,
        role_id: Id<RoleMarker>,
    ) -> Result<(), String> {
        if role_id == self.guild_id.cast::<RoleMarker>() {
            return Err(format!(
                "{tool}: @everyone can't be assigned, removed or deleted."
            ));
        }
        // Fast path: snapshot hit.
        let info_opt = self.snapshot.roles.get(&role_id).cloned();
        let info = match info_opt {
            Some(i) => i,
            None => {
                // Slow path: snapshot may be stale (role created seconds ago).
                // Verify over HTTP once instead of false-denying.
                match ctx.http.roles(self.guild_id).await {
                    Ok(resp) => match resp.model().await {
                        Ok(roles) => {
                            let found = roles.iter().find(|r| r.id == role_id);
                            match found {
                                Some(r) => crate::access::RoleInfo {
                                    name: r.name.clone(),
                                    position: r.position,
                                    permissions: r.permissions,
                                    managed: r.managed,
                                },
                                None => {
                                    return Err(format!(
                                        "{tool}: role {role_id} does not exist in this server. Use list_roles to find the right ID and retry once."
                                    ));
                                }
                            }
                        }
                        Err(_) => {
                            return Err(format!(
                                "{tool}: role {role_id} is not in my cached view and I couldn't refresh it. Use list_roles to re-ground and retry once."
                            ));
                        }
                    },
                    Err(_) => {
                        return Err(format!(
                            "{tool}: role {role_id} is not in my cached view and I couldn't refresh it. Use list_roles to re-ground and retry once."
                        ));
                    }
                }
            }
        };
        if info.managed {
            return Err(format!(
                "{tool}: role '{}' is managed by an integration/bot and can't be changed manually.",
                info.name
            ));
        }
        let auth = &self.authority;
        if auth.bypass {
            return Ok(());
        }
        if !auth.outranks(info.position) {
            return deny(format!(
                "role '{}' (position {}) is at or above the requester's highest role. The requester's top role must be HIGHER — ask a higher moderator/owner, or move the requester's role above it in Server Settings → Roles.",
                info.name, info.position
            ));
        }
        if tool == "assign_role"
            && !auth.is_guild_owner
            && !auth.permissions.contains(info.permissions)
        {
            return deny(format!(
                "role '{}' grants permissions the requester doesn't have. They can't grant what they don't hold — ask an admin who has those permissions.",
                info.name
            ));
        }
        Ok(())
    }

    async fn check_member_target(
        &self,
        ctx: &Context,
        tool: &str,
        target: Id<UserMarker>,
    ) -> Result<(), String> {
        let auth = &self.authority;
        // set_nickname on self and remove_timeout on self are harmless and
        // explicitly allowed (self-unnick, self-unmute requests).
        if target == auth.user_id
            && matches!(
                tool,
                "set_nickname" | "remove_timeout" | "assign_role" | "remove_role"
            )
        {
            return Ok(());
        }
        if target == self.snapshot.owner_id && !auth.is_guild_owner {
            return deny("the server owner can't be targeted by a non-owner. Only the server owner (or bot owner) can moderate them.");
        }
        // Always prefer a fresh HTTP fetch for the TARGET's roles too — the
        // member cache may be missing (no GUILD_MEMBERS intent) or stale.
        let roles = match ctx.http.guild_member(self.guild_id, target).await {
            Ok(resp) => resp.model().await.ok().map(|m| m.roles),
            Err(_) => ctx
                .cache
                .member(self.guild_id, target)
                .map(|m| m.roles().to_vec()),
        };
        let Some(roles) = roles else {
            if tool == "ban_member" {
                // Banning by ID works even when the user isn't (or no longer
                // is) a member — allow it through.
                return Ok(());
            }
            return Err(format!(
                "{tool}: user {target} is not a member of this server (or I can't see them). Use search_members to find the right ID; if they left, ban_member by ID still works."
            ));
        };
        let pos = self.snapshot.top_position(&roles);
        if pos == 0 && roles.iter().any(|r| !self.snapshot.roles.contains_key(r)) {
            // Target has roles we don't know — our snapshot is stale, not the
            // requester. Say so instead of a misleading hierarchy deny.
            tracing::debug!(target = %target, "Target has unknown roles, hierarchy check may be stale");
        }
        if !auth.outranks(pos) {
            return deny(format!(
                "<@{target}>'s highest role (position {pos}) is at or above the requester's. Discord requires the requester's top role to be strictly HIGHER — ask a higher moderator/owner."
            ));
        }
        Ok(())
    }

    async fn ensure_channel_in_guild(
        &self,
        ctx: &Context,
        channel: Id<ChannelMarker>,
    ) -> Result<(), String> {
        let gid = match ctx.cache.channel(channel) {
            Some(c) => c.guild_id,
            None => match ctx.http.channel(channel).await {
                Ok(resp) => resp.model().await.ok().and_then(|c| c.guild_id),
                Err(_) => {
                    return Err(format!(
                        "Channel {channel} was not found. Use list_channels to get valid IDs."
                    ))
                }
            },
        };
        if gid != Some(self.guild_id) {
            return Err(format!(
                "Channel {channel} is not in this server. Use list_channels to get valid IDs."
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mutating_tool_requires_a_permission() {
        for tool in crate::tools::tool_names() {
            if !crate::tools::is_read_only(tool) {
                assert!(
                    required_permission(tool).is_some(),
                    "{tool} has no permission requirement"
                );
            }
        }
    }

    #[test]
    fn labels_are_human() {
        assert_eq!(permission_label(Permissions::BAN_MEMBERS), "Ban Members");
        assert!(
            permission_label(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).contains('+')
        );
    }
}
