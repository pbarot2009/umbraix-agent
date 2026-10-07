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
}

pub fn required_permission(tool: &str) -> Option<Permissions> {
    Some(match tool {
        "list_bans" => Permissions::BAN_MEMBERS,
        "get_messages" => Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
        "send_message" => Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
        "kick_member" => Permissions::KICK_MEMBERS,
        "ban_member" | "unban_user" => Permissions::BAN_MEMBERS,
        "timeout_member" | "remove_timeout" => Permissions::MODERATE_MEMBERS,
        "purge_messages" => Permissions::MANAGE_MESSAGES,
        "set_nickname" => Permissions::MANAGE_NICKNAMES,
        "create_channel" | "rename_channel" | "delete_channel" | "set_slowmode" | "set_topic" => {
            Permissions::MANAGE_CHANNELS
        }
        "create_role" | "assign_role" | "remove_role" | "delete_role" => Permissions::MANAGE_ROLES,
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
    )
}

fn targets_role(tool: &str) -> bool {
    matches!(tool, "assign_role" | "remove_role" | "delete_role")
}

pub fn permission_label(p: Permissions) -> String {
    const NAMES: &[(Permissions, &str)] = &[
        (Permissions::BAN_MEMBERS, "Ban Members"),
        (Permissions::KICK_MEMBERS, "Kick Members"),
        (Permissions::MODERATE_MEMBERS, "Timeout Members"),
        (Permissions::MANAGE_MESSAGES, "Manage Messages"),
        (Permissions::MANAGE_NICKNAMES, "Manage Nicknames"),
        (Permissions::MANAGE_CHANNELS, "Manage Channels"),
        (Permissions::MANAGE_ROLES, "Manage Roles"),
        (Permissions::VIEW_CHANNEL, "View Channel"),
        (Permissions::READ_MESSAGE_HISTORY, "Read Message History"),
        (Permissions::SEND_MESSAGES, "Send Messages"),
        (Permissions::MENTION_EVERYONE, "Mention @everyone"),
        (Permissions::ADMINISTRATOR, "Administrator"),
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
                return deny(format!(
                    "the requester lacks **{}** in this server.",
                    permission_label(req - (req & auth.permissions))
                ));
            }
            if let Some(ch) = channel {
                if !auth.bypass && !auth.is_guild_owner {
                    if let Ok(cp) = ctx.cache.permissions().in_channel(auth.user_id, ch) {
                        if !cp.contains(req) {
                            return deny(format!(
                                "the requester lacks **{}** in <#{ch}>.",
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
                return deny("the requester can't mention @everyone/@here.");
            }
        }

        if targets_role(tool) {
            let role_id = parse_role_id(args, "role_id").map_err(|e| format!("{tool}: {e}"))?;
            self.check_role(tool, role_id)?;
        }

        if targets_member(tool) && !auth.bypass {
            let target = parse_user_id(args, "user_id").map_err(|e| format!("{tool}: {e}"))?;
            self.check_member_target(ctx, tool, target).await?;
        }
        Ok(())
    }

    fn check_role(&self, tool: &str, role_id: Id<RoleMarker>) -> Result<(), String> {
        if role_id == self.guild_id.cast::<RoleMarker>() {
            return Err(format!(
                "{tool}: @everyone can't be assigned, removed or deleted."
            ));
        }
        let Some(info) = self.snapshot.roles.get(&role_id) else {
            return Err(format!(
                "{tool}: role {role_id} does not exist in this server. Use list_roles to find the right ID."
            ));
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
                "role '{}' is at or above the requester's highest role.",
                info.name
            ));
        }
        if tool == "assign_role"
            && !auth.is_guild_owner
            && !auth.permissions.contains(info.permissions)
        {
            return deny(format!(
                "role '{}' grants permissions the requester doesn't have.",
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
        if target == auth.user_id {
            return Ok(());
        }
        if target == self.snapshot.owner_id && !auth.is_guild_owner {
            return deny("the server owner can't be targeted.");
        }
        let roles = match ctx.cache.member(self.guild_id, target) {
            Some(m) => Some(m.roles().to_vec()),
            None => match ctx.http.guild_member(self.guild_id, target).await {
                Ok(resp) => resp.model().await.ok().map(|m| m.roles),
                Err(_) => None,
            },
        };
        let Some(roles) = roles else {
            if tool == "ban_member" {
                return Ok(());
            }
            return Err(format!(
                "{tool}: user {target} is not a member of this server. Use search_members to find them."
            ));
        };
        let pos = self.snapshot.top_position(&roles);
        if !auth.outranks(pos) {
            return deny(format!(
                "<@{target}>'s highest role is at or above the requester's."
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
