use std::{collections::HashMap, time::Duration};
use twilight_model::{
    guild::Permissions,
    id::{
        marker::{GuildMarker, RoleMarker, UserMarker},
        Id,
    },
};

use crate::{
    context::Context,
    error::BotError,
    storage::{ApiKey, KeyOwner},
};

/// Whose Gemini key powers a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// Bot owner using the operator key from `.env` (no restrictions).
    Owner,
    /// The requester's personal BYOK key (works in any server).
    User,
    /// The server's BYOK key (server owner + `allow_agent` role).
    Server,
}

impl KeySource {
    pub const fn label(self) -> &'static str {
        match self {
            KeySource::Owner => "owner",
            KeySource::User => "personal key",
            KeySource::Server => "server key",
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            KeySource::Owner => "owner",
            KeySource::User => "user",
            KeySource::Server => "server",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Grant {
    pub source: KeySource,
    pub api_key: ApiKey,
    pub model: String,
    pub key_owner: Option<KeyOwner>,
    pub cooldown: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    BotDisabled,
    NoAccess {
        server_has_key: bool,
        role_id: Option<u64>,
        user_has_key: bool,
        user_byok_blocked: bool,
    },
}

impl Denied {
    pub fn render(&self, prefix: &str, role_name: &str) -> (String, String) {
        match self {
            Denied::BotDisabled => (
                "Umbraix is disabled here".into(),
                format!("The server owner turned the agent off. They can re-enable it with `{prefix}config enabled on`."),
            ),
            Denied::NoAccess {
                server_has_key,
                role_id,
                user_has_key,
                user_byok_blocked,
            } => {
                let mut lines = vec!["You don't have access to the AI features here yet. Options:".to_string()];
                if *user_has_key && *user_byok_blocked {
                    lines.push("• This server has disabled personal keys, so your own key can't be used here.".into());
                } else {
                    lines.push(format!(
                        "• **Bring your own key** — run `{prefix}byok-user` (or `/byok-user`) to add your free Gemini key; it then works in every server I'm in."
                    ));
                }
                if *server_has_key {
                    let role = role_id
                        .map(|r| format!("<@&{r}>"))
                        .unwrap_or_else(|| format!("`{role_name}`"));
                    lines.push(format!("• **Ask for the {role} role** — this server provides a shared key for members with it."));
                } else {
                    lines.push(format!(
                        "• **Server key** — the server owner can run `{prefix}byok-server` to give staff shared access."
                    ));
                }
                ("Access required".into(), lines.join("\n"))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoleInfo {
    pub name: String,
    pub position: i64,
    pub permissions: Permissions,
    pub managed: bool,
}

/// Point-in-time view of a guild's owner and roles, used for access
/// decisions and permission/hierarchy checks during one request.
#[derive(Debug, Clone)]
pub struct GuildSnapshot {
    pub guild_id: Id<GuildMarker>,
    pub owner_id: Id<UserMarker>,
    pub roles: HashMap<Id<RoleMarker>, RoleInfo>,
}

impl GuildSnapshot {
    pub async fn load(ctx: &Context, guild_id: Id<GuildMarker>) -> Result<Self, BotError> {
        let cached_owner = ctx.cache.guild(guild_id).map(|g| g.owner_id());
        let owner_id = match cached_owner {
            Some(o) => o,
            None => ctx.http.guild(guild_id).await?.model().await?.owner_id,
        };

        let mut roles = HashMap::new();
        if let Some(ids) = ctx.cache.guild_roles(guild_id) {
            for id in ids.iter() {
                if let Some(r) = ctx.cache.role(*id) {
                    let r = r.resource();
                    roles.insert(
                        r.id,
                        RoleInfo {
                            name: r.name.clone(),
                            position: r.position,
                            permissions: r.permissions,
                            managed: r.managed,
                        },
                    );
                }
            }
        }
        if roles.is_empty() || !roles.contains_key(&guild_id.cast()) {
            tracing::debug!(guild = %guild_id, "Role cache miss, fetching roles over HTTP");
            roles.clear();
            for r in ctx.http.roles(guild_id).await?.model().await? {
                roles.insert(
                    r.id,
                    RoleInfo {
                        name: r.name,
                        position: r.position,
                        permissions: r.permissions,
                        managed: r.managed,
                    },
                );
            }
        } else {
            // Cache had *something* but may still be stale (role created /
            // deleted moments ago and the gateway event hasn't arrived yet).
            // If the cached set looks suspiciously small compared to a fresh
            // fetch we prefer HTTP. To avoid an extra HTTP call on every
            // request, callers use `ensure_role_present` / `refresh` when a
            // specific role lookup fails instead of always refetching here.
        }
        Ok(Self {
            guild_id,
            owner_id,
            roles,
        })
    }

    /// Re-fetch roles + owner over HTTP and replace the cached view.
    /// Call when a role lookup fails — the in-memory cache may be stale.
    pub async fn refresh(&mut self, ctx: &Context) -> Result<(), BotError> {
        let guild = ctx.http.guild(self.guild_id).await?.model().await?;
        self.owner_id = guild.owner_id;
        let mut roles = HashMap::new();
        for r in ctx.http.roles(self.guild_id).await?.model().await? {
            roles.insert(
                r.id,
                RoleInfo {
                    name: r.name,
                    position: r.position,
                    permissions: r.permissions,
                    managed: r.managed,
                },
            );
        }
        self.roles = roles;
        Ok(())
    }

    /// If `role_id` isn't in this snapshot, refresh once over HTTP.
    /// Returns true when the role exists after the refresh.
    /// This fixes the classic false-deny: role created/renamed seconds ago,
    /// gateway cache hasn't caught up, guard says "role does not exist".
    pub async fn ensure_role_present(&mut self, ctx: &Context, role_id: Id<RoleMarker>) -> bool {
        if self.roles.contains_key(&role_id) {
            return true;
        }
        tracing::debug!(guild = %self.guild_id, role = %role_id, "Role missing from snapshot, refreshing over HTTP");
        if self.refresh(ctx).await.is_ok() {
            return self.roles.contains_key(&role_id);
        }
        false
    }

    pub fn top_position(&self, member_roles: &[Id<RoleMarker>]) -> i64 {
        member_roles
            .iter()
            .filter_map(|r| self.roles.get(r).map(|i| i.position))
            .max()
            .unwrap_or(0)
    }

    pub fn permissions(
        &self,
        user_id: Id<UserMarker>,
        member_roles: &[Id<RoleMarker>],
    ) -> Permissions {
        if user_id == self.owner_id {
            return Permissions::all();
        }
        let mut perms = self
            .roles
            .get(&self.guild_id.cast())
            .map(|r| r.permissions)
            .unwrap_or_else(Permissions::empty);
        for r in member_roles {
            if let Some(info) = self.roles.get(r) {
                perms |= info.permissions;
            }
        }
        if perms.contains(Permissions::ADMINISTRATOR) {
            Permissions::all()
        } else {
            perms
        }
    }

    pub fn authority(
        &self,
        user_id: Id<UserMarker>,
        member_roles: &[Id<RoleMarker>],
        bypass: bool,
    ) -> Authority {
        Authority {
            user_id,
            bypass,
            is_guild_owner: user_id == self.owner_id,
            permissions: self.permissions(user_id, member_roles),
            top_position: self.top_position(member_roles),
        }
    }

    pub fn find_role_by_name(&self, name: &str) -> Option<Id<RoleMarker>> {
        self.roles
            .iter()
            .find(|(_, r)| r.name.eq_ignore_ascii_case(name))
            .map(|(id, _)| *id)
    }
}

/// The requester's effective power in a guild for this request.
#[derive(Debug, Clone)]
pub struct Authority {
    pub user_id: Id<UserMarker>,
    /// Bot owner: skip Discord permission checks (Discord still enforces the bot's own perms).
    pub bypass: bool,
    pub is_guild_owner: bool,
    pub permissions: Permissions,
    pub top_position: i64,
}

impl Authority {
    pub fn has(&self, required: Permissions) -> bool {
        self.bypass || self.permissions.contains(required)
    }

    /// May act on something whose highest role sits at `position`.
    pub fn outranks(&self, position: i64) -> bool {
        self.bypass || self.is_guild_owner || self.top_position > position
    }
}

/// `(guild, requester_is_guild_owner, requester_role_ids)`.
pub type GuildRequester<'a> = (Id<GuildMarker>, bool, &'a [Id<RoleMarker>]);

/// Fetch authoritative member roles over HTTP, falling back to the gateway
/// payload when the fetch fails.
///
/// The gateway `member.roles` payload can be empty/stale (missing
/// `GUILD_MEMBERS` intent, uncached member, role changed seconds ago).
/// Trusting it blindly is the #1 cause of "I have the role but the bot says
/// I don't". Always try HTTP first for access decisions.
pub async fn fresh_member_roles(
    ctx: &Context,
    guild_id: Id<GuildMarker>,
    user_id: Id<UserMarker>,
    fallback: &[Id<RoleMarker>],
) -> Vec<Id<RoleMarker>> {
    match ctx.http.guild_member(guild_id, user_id).await {
        Ok(resp) => match resp.model().await {
            Ok(m) => m.roles,
            Err(e) => {
                tracing::debug!(guild = %guild_id, user = %user_id, error = %e, "guild_member decode failed, using gateway roles");
                fallback.to_vec()
            }
        },
        Err(e) => {
            tracing::debug!(guild = %guild_id, user = %user_id, error = %e, "guild_member fetch failed, using gateway roles");
            fallback.to_vec()
        }
    }
}

/// Resolve the effective access role: the configured `allow_role_id` when it
/// still exists, otherwise a role with the configured name (auto-heal for
/// deleted/recreated roles).
///
/// Returns `(effective_role_id, healed)` where `healed` means the stored ID
/// is stale and the caller should persist the healed ID.
pub fn effective_allow_role(
    allow_role_id: Option<u64>,
    snapshot: Option<&GuildSnapshot>,
    role_name: &str,
) -> (Option<u64>, bool) {
    let Some(snap) = snapshot else {
        return (allow_role_id, false);
    };
    match allow_role_id.map(Id::<RoleMarker>::new) {
        Some(id) if snap.roles.contains_key(&id) => (Some(id.get()), false),
        Some(_) => {
            // Stored ID points at a deleted role — try to heal by name.
            match snap.find_role_by_name(role_name) {
                Some(healed) => (Some(healed.get()), true),
                None => (allow_role_id, false),
            }
        }
        None => match snap.find_role_by_name(role_name) {
            Some(found) => (Some(found.get()), true),
            None => (None, false),
        },
    }
}

/// Decide which key (if any) serves this request.
///
/// Order: bot owner → personal key (unless the server disabled personal
/// keys) → server key (server owner or `allow_agent` role) → denied.
///
/// Pass `snapshot` when the caller already loaded one (the hot path does):
/// it is used to auto-heal a stale `allow_role_id` (role deleted +
/// recreated) and to recompute guild ownership authoritatively instead of
/// trusting a possibly-stale boolean.
pub async fn resolve(
    ctx: &Context,
    user_id: u64,
    guild: Option<GuildRequester<'_>>,
) -> Result<Result<Grant, Denied>, BotError> {
    resolve_with_snapshot(ctx, user_id, guild, None).await
}

pub async fn resolve_with_snapshot(
    ctx: &Context,
    user_id: u64,
    guild: Option<GuildRequester<'_>>,
    snapshot: Option<&GuildSnapshot>,
) -> Result<Result<Grant, Denied>, BotError> {
    let cfg = match guild {
        Some((gid, _, _)) => Some(ctx.store.guild_config(gid.get()).await?),
        None => None,
    };
    let model = cfg
        .as_ref()
        .and_then(|c| c.model.clone())
        .unwrap_or_else(|| ctx.config.gemini_model.clone());
    let cooldown = cfg
        .as_ref()
        .and_then(|c| c.cooldown_secs)
        .map(Duration::from_secs);

    if ctx.is_owner(user_id) {
        return Ok(Ok(Grant {
            source: KeySource::Owner,
            api_key: ctx.owner_key.clone(),
            model,
            key_owner: None,
            cooldown: None,
        }));
    }

    if let Some(c) = &cfg {
        if !c.enabled {
            return Ok(Err(Denied::BotDisabled));
        }
    }

    let user_byok_allowed = cfg.as_ref().map(|c| c.allow_user_byok).unwrap_or(true);
    let user_key = ctx.store.get_key(KeyOwner::User(user_id)).await?;
    if let Some(k) = &user_key {
        if user_byok_allowed {
            return Ok(Ok(Grant {
                source: KeySource::User,
                api_key: k.api_key.clone(),
                model,
                key_owner: Some(KeyOwner::User(user_id)),
                cooldown,
            }));
        }
    }

    let mut server_has_key = false;
    let mut role_id = None;
    if let (Some((gid, is_guild_owner_flag, member_roles)), Some(c)) = (guild, &cfg) {
        // Authoritative ownership when we have a snapshot; otherwise trust flag.
        let is_guild_owner = snapshot
            .map(|s| s.owner_id.get() == user_id)
            .unwrap_or(is_guild_owner_flag);
        let (effective_role, healed) =
            effective_allow_role(c.allow_role_id, snapshot, &ctx.config.allow_agent_role_name);
        role_id = effective_role.or(c.allow_role_id);
        // Persist the healed role ID in the background so the next request
        // doesn't need to heal again (deleted role recreated with new ID,
        // or role existed by name but config was never set).
        if healed {
            if let Some(healed_id) = effective_role {
                let gid_u64 = gid.get();
                let ctx_clone = ctx.clone();
                tokio::spawn(async move {
                    let _ = ctx_clone
                        .store
                        .update_guild_config(gid_u64, |cfg| {
                            cfg.allow_role_id = Some(healed_id);
                        })
                        .await;
                });
                tracing::info!(guild = %gid, healed_role = healed_id, "Auto-healed stale allow_role_id");
            }
        }
        if let Some(k) = ctx.store.get_key(KeyOwner::Guild(gid.get())).await? {
            server_has_key = true;
            let has_role =
                effective_role.is_some_and(|r| member_roles.iter().any(|m| m.get() == r));
            if is_guild_owner || has_role {
                return Ok(Ok(Grant {
                    source: KeySource::Server,
                    api_key: k.api_key.clone(),
                    model,
                    key_owner: Some(KeyOwner::Guild(gid.get())),
                    cooldown,
                }));
            }
        }
    }

    Ok(Err(Denied::NoAccess {
        server_has_key,
        role_id,
        user_has_key: user_key.is_some(),
        user_byok_blocked: user_key.is_some() && !user_byok_allowed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> GuildSnapshot {
        let gid: Id<GuildMarker> = Id::new(1);
        let mut roles = HashMap::new();
        roles.insert(
            gid.cast(),
            RoleInfo {
                name: "@everyone".into(),
                position: 0,
                permissions: Permissions::SEND_MESSAGES,
                managed: false,
            },
        );
        roles.insert(
            Id::new(10),
            RoleInfo {
                name: "Mod".into(),
                position: 5,
                permissions: Permissions::KICK_MEMBERS,
                managed: false,
            },
        );
        roles.insert(
            Id::new(11),
            RoleInfo {
                name: "Admin".into(),
                position: 9,
                permissions: Permissions::ADMINISTRATOR,
                managed: false,
            },
        );
        GuildSnapshot {
            guild_id: gid,
            owner_id: Id::new(100),
            roles,
        }
    }

    #[test]
    fn permissions_union_and_admin() {
        let s = snapshot();
        let p = s.permissions(Id::new(5), &[Id::new(10)]);
        assert!(p.contains(Permissions::KICK_MEMBERS | Permissions::SEND_MESSAGES));
        assert!(!p.contains(Permissions::BAN_MEMBERS));
        assert_eq!(
            s.permissions(Id::new(5), &[Id::new(11)]),
            Permissions::all()
        );
        assert_eq!(s.permissions(Id::new(100), &[]), Permissions::all());
    }

    #[test]
    fn hierarchy() {
        let s = snapshot();
        let mod_auth = s.authority(Id::new(5), &[Id::new(10)], false);
        assert!(mod_auth.outranks(3));
        assert!(!mod_auth.outranks(5));
        assert!(!mod_auth.outranks(9));
        let owner = s.authority(Id::new(100), &[], false);
        assert!(owner.outranks(100));
        assert_eq!(s.find_role_by_name("mod"), Some(Id::new(10)));
    }

    #[test]
    fn denied_message_mentions_commands() {
        let d = Denied::NoAccess {
            server_has_key: true,
            role_id: Some(42),
            user_has_key: false,
            user_byok_blocked: false,
        };
        let (_, body) = d.render("!", "allow_agent");
        assert!(body.contains("!byok-user"));
        assert!(body.contains("<@&42>"));
    }
}
