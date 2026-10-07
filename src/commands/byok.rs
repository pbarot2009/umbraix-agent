use std::time::{Duration, Instant};
use twilight_http::request::AuditLogReason;
use twilight_model::{
    channel::{
        message::{
            component::{ActionRow, Button, ButtonStyle, Label, TextInput, TextInputStyle},
            Component, Embed,
        },
        Message,
    },
    guild::Permissions,
    id::{
        marker::{GuildMarker, RoleMarker, UserMarker},
        Id,
    },
};

use super::Invocation;
use crate::{
    access::{self, GuildSnapshot},
    brand::{self, Tone},
    context::{ByokScope, Context, PendingByok},
    discord::reply::{self, Responder},
    error::BotError,
    gemini::GeminiError,
    storage::{ApiKey, AuditEntry, KeyOwner},
    utils,
};

pub const BUTTON_PREFIX: &str = "umbraix:byok-btn:";
pub const MODAL_PREFIX: &str = "umbraix:byok-modal:";
pub const KEY_INPUT_ID: &str = "api_key";
const AI_STUDIO_URL: &str = "https://aistudio.google.com/apikey";

fn scope_suffix(scope: ByokScope) -> String {
    match scope {
        ByokScope::User => "user".into(),
        ByokScope::Server(g) => format!("server:{g}"),
    }
}

pub fn parse_scope(suffix: &str) -> Option<ByokScope> {
    if suffix == "user" {
        return Some(ByokScope::User);
    }
    let gid: u64 = suffix.strip_prefix("server:")?.parse().ok()?;
    Id::<GuildMarker>::new_checked(gid).map(ByokScope::Server)
}

#[allow(deprecated)]
fn key_modal_components(scope: ByokScope) -> Vec<Component> {
    let description = match scope {
        ByokScope::User => "Used only for your requests. Stored encrypted.",
        ByokScope::Server(_) => "Shared by the server owner + access role. Stored encrypted.",
    };
    vec![Component::Label(Label {
        id: None,
        label: "Gemini API key".into(),
        description: Some(description.into()),
        component: Box::new(Component::TextInput(TextInput {
            id: None,
            custom_id: KEY_INPUT_ID.into(),
            label: None,
            max_length: Some(200),
            min_length: Some(20),
            placeholder: Some("AIza…".into()),
            required: Some(true),
            style: TextInputStyle::Short,
            value: None,
        })),
    })]
}

fn buttons(scope: ByokScope) -> Vec<Component> {
    vec![Component::ActionRow(ActionRow {
        id: None,
        components: vec![
            Component::Button(Button {
                id: None,
                custom_id: Some(format!("{BUTTON_PREFIX}{}", scope_suffix(scope))),
                disabled: false,
                emoji: None,
                label: Some("Enter API key securely".into()),
                style: ButtonStyle::Primary,
                url: None,
                sku_id: None,
            }),
            Component::Button(Button {
                id: None,
                custom_id: None,
                disabled: false,
                emoji: None,
                label: Some("Get a free key".into()),
                style: ButtonStyle::Link,
                url: Some(AI_STUDIO_URL.into()),
                sku_id: None,
            }),
        ],
    })]
}

pub async fn open_modal(
    ctx: &Context,
    responder: &Responder,
    scope: ByokScope,
) -> Result<(), BotError> {
    let title = match scope {
        ByokScope::User => "Your Gemini API key",
        ByokScope::Server(_) => "Server Gemini API key",
    };
    responder
        .modal(
            ctx,
            format!("{MODAL_PREFIX}{}", scope_suffix(scope)),
            title,
            key_modal_components(scope),
        )
        .await
}

pub async fn start_user(ctx: &Context, inv: Invocation) {
    start(ctx, inv, ByokScope::User).await
}

pub async fn start_server(ctx: &Context, inv: Invocation) {
    let Some(gid) = inv.guild_id else { return };
    start(ctx, inv, ByokScope::Server(gid)).await
}

async fn start(ctx: &Context, inv: Invocation, scope: ByokScope) {
    if inv.is_slash {
        if let Err(e) = open_modal(ctx, &inv.responder, scope).await {
            inv.responder.error(ctx, &e, None, &inv.request_id).await;
        }
        return;
    }

    let secs = ctx.config.byok_session_secs;
    // Sweep expired sessions opportunistically so the map can't leak.
    let now = Instant::now();
    ctx.byok_sessions.retain(|_, s| s.expires > now);
    ctx.byok_sessions.insert(
        inv.user_id.get(),
        PendingByok {
            scope,
            expires: Instant::now() + Duration::from_secs(secs),
        },
    );
    let what = match scope {
        ByokScope::User => {
            "your **personal** key — it will work for you in every server I'm in".to_string()
        }
        ByokScope::Server(g) => {
            let name = ctx
                .cache
                .guild(g)
                .map(|g| g.name().to_string())
                .unwrap_or_else(|| g.to_string());
            format!(
                "the **server** key for **{name}** — the owner and the `{}` role will share it",
                ctx.config.allow_agent_role_name
            )
        }
    };
    let body = format!(
        "Let's set up {what}.\n\n\
         **Recommended:** press **Enter API key securely** (private form).\n\
         **Or:** paste the key here as a message within {} — note I can't delete your DM messages, so delete it yourself afterwards.\n\n\
         • Get a free key at {AI_STUDIO_URL}\n\
         • The key is verified with Google, encrypted with AES-256-GCM, and never shown again.\n\
         • Remove it anytime with `{}byok-remove`.",
        utils::format_duration(secs),
        ctx.prefix()
    );
    let embed = brand::embed(Tone::Primary, "Bring your own key", &body);
    match reply::dm(ctx, inv.user_id, embed, buttons(scope)).await {
        Ok(_) => {
            tracing::info!(user = %inv.user_id, ?scope, "BYOK DM session started");
            if inv.guild_id.is_some() {
                inv.responder
                    .info(
                        ctx,
                        Tone::Info,
                        "Check your DMs",
                        "I sent you a private message to collect the key securely.",
                        true,
                    )
                    .await;
            }
        }
        Err(e) => {
            ctx.byok_sessions.remove(&inv.user_id.get());
            tracing::info!(user = %inv.user_id, error = %e, "Could not DM user for BYOK");
            let slash = match scope {
                ByokScope::User => "/byok-user",
                ByokScope::Server(_) => "/byok-server",
            };
            inv.responder
                .info(
                    ctx,
                    Tone::Warn,
                    "I couldn't DM you",
                    &format!(
                        "Your DMs seem closed. Either enable **Direct Messages** for this server (Privacy Settings), \
                         or use the slash command **{slash}** which opens a private form right here. \
                         **Never paste API keys in public channels.**"
                    ),
                    true,
                )
                .await;
        }
    }
}

/// A user typed something in DMs while a BYOK session is pending.
pub async fn handle_dm_text(ctx: &Context, msg: &Message) -> bool {
    let uid = msg.author.id.get();
    let Some(pending) = ctx.byok_sessions.get(&uid).map(|p| *p) else {
        return false;
    };
    if pending.expires < Instant::now() {
        ctx.byok_sessions.remove(&uid);
        return false;
    }
    let raw = msg.content.trim();
    if raw.is_empty() || raw.starts_with(ctx.prefix()) {
        return false;
    }
    let responder = Responder::channel(msg.channel_id, Some(msg.id));
    let embed = match submit_key(ctx, msg.author.id, pending.scope, raw).await {
        Ok(mut e) => {
            e.description = e.description.map(|d| {
                format!("{d}\n\n🧹 Please **delete your message** containing the key — I can't delete messages in DMs.")
            });
            e
        }
        Err(e) => error_embed(ctx, &e),
    };
    if let Err(e) = responder.embed(ctx, embed, false).await {
        tracing::warn!(error = %e, "Failed to reply to BYOK DM");
    }
    true
}

pub async fn submit_from_modal(
    ctx: &Context,
    responder: &Responder,
    user_id: Id<UserMarker>,
    scope: ByokScope,
    raw: &str,
) {
    if let Err(e) = responder.defer(ctx, true).await {
        tracing::warn!(error = %e, "Failed to defer modal submit");
    }
    let embed = match submit_key(ctx, user_id, scope, raw).await {
        Ok(e) => e,
        Err(e) => error_embed(ctx, &e),
    };
    if let Err(e) = responder.embed(ctx, embed, true).await {
        tracing::warn!(error = %e, "Failed to reply to BYOK modal");
    }
}

fn error_embed(ctx: &Context, e: &BotError) -> Embed {
    brand::embed(Tone::Error, "Key not saved", &e.hint(None, ctx.prefix()))
}

fn sanitize_key(raw: &str) -> Result<ApiKey, BotError> {
    let key = raw
        .trim()
        .trim_matches(|c| c == '`' || c == '"' || c == '\'')
        .trim();
    if key.len() < 20 || key.len() > 200 || key.contains(char::is_whitespace) {
        return Err(BotError::User(
            "That doesn't look like a Gemini API key. Copy it from https://aistudio.google.com/apikey (it usually starts with `AIza`).".into(),
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(BotError::User(
            "The key contains unexpected characters. Paste only the key itself.".into(),
        ));
    }
    Ok(ApiKey::new(key))
}

async fn submit_key(
    ctx: &Context,
    user_id: Id<UserMarker>,
    scope: ByokScope,
    raw: &str,
) -> Result<Embed, BotError> {
    let key = sanitize_key(raw)?;

    let snapshot = match scope {
        ByokScope::User => None,
        ByokScope::Server(gid) => {
            let s = GuildSnapshot::load(ctx, gid).await.map_err(|_| {
                BotError::User("I can't see that server anymore — am I still in it?".into())
            })?;
            if s.owner_id != user_id && !ctx.is_owner(user_id.get()) {
                return Err(BotError::User(
                    "Only the server owner can set the server key.".into(),
                ));
            }
            Some(s)
        }
    };

    let mut validation_note = String::new();
    match ctx.gemini.validate_key(&key).await {
        Ok(()) => {}
        Err(GeminiError::RateLimited { .. }) | Err(GeminiError::QuotaExhausted(_)) => {
            validation_note = "\n⚠ Google reports this key is currently rate-limited/out of quota; it was saved but may fail until the quota resets.".into();
        }
        Err(GeminiError::InvalidKey) => {
            return Err(BotError::User(
                "Google rejected this key (invalid, revoked or leaked). Create a new one at https://aistudio.google.com/apikey and try again.".into(),
            ))
        }
        Err(GeminiError::PermissionDenied(d)) => {
            return Err(BotError::User(format!(
                "This key can't use the Gemini API ({d}). Make sure it's a Google AI Studio key with the Generative Language API enabled."
            )))
        }
        Err(e) if e.is_retryable() => {
            return Err(BotError::User(format!(
                "I couldn't reach Google to verify the key ({e}). Nothing was saved — please try again in a minute."
            )))
        }
        Err(e) => {
            return Err(BotError::User(format!("Google didn't accept this key: {e}")));
        }
    }

    let owner = match scope {
        ByokScope::User => KeyOwner::User(user_id.get()),
        ByokScope::Server(g) => KeyOwner::Guild(g.get()),
    };
    ctx.store.set_key(owner, &key, user_id.get()).await?;
    ctx.gemini.unpause(&key);
    ctx.byok_sessions.remove(&user_id.get());

    let (title, body) = match (scope, snapshot) {
        (ByokScope::Server(gid), Some(snap)) => {
            let role_line = match ensure_access_role(ctx, gid, &snap).await {
                Ok((role, created)) => format!(
                    "Access role: <@&{role}>{}. Give it to staff who should use the agent on this key.",
                    if created { " (created)" } else { "" }
                ),
                Err(e) => {
                    tracing::warn!(guild = %gid, error = %e, "Could not create access role");
                    format!(
                        "⚠ I couldn't create the `{}` role ({}). Give me **Manage Roles**, then run `{}byok-server` again, or pick an existing role with `{}config role @role`.",
                        ctx.config.allow_agent_role_name,
                        utils::truncate_chars(&e.to_string(), 120),
                        ctx.prefix(),
                        ctx.prefix()
                    )
                }
            };
            (
                "Server key saved",
                format!(
                    "Key `{}` verified and stored encrypted for this server.\n{role_line}\n\nThe server owner can always use it. Members with their own personal key keep using theirs.{validation_note}",
                    key.hint()
                ),
            )
        }
        _ => (
            "Personal key saved",
            format!(
                "Key `{}` verified and stored encrypted.\nYou can now use `{}ai` (or `/ai`) in any server I'm in, within your own Discord permissions.{validation_note}",
                key.hint(),
                ctx.prefix()
            ),
        ),
    };

    tracing::info!(user = %user_id, ?scope, hint = %key.hint(), "BYOK key stored");
    let ctx2 = ctx.clone();
    let guild_id = match scope {
        ByokScope::Server(g) => Some(g.get()),
        ByokScope::User => None,
    };
    tokio::spawn(async move {
        let _ = ctx2
            .store
            .audit(AuditEntry {
                guild_id,
                user_id: user_id.get(),
                key_source: None,
                action: if guild_id.is_some() {
                    "byok_set_server"
                } else {
                    "byok_set_user"
                }
                .into(),
                detail: String::new(),
                success: true,
            })
            .await;
    });
    Ok(brand::embed(Tone::Success, title, &body))
}

async fn ensure_access_role(
    ctx: &Context,
    gid: Id<GuildMarker>,
    snap: &GuildSnapshot,
) -> Result<(Id<RoleMarker>, bool), BotError> {
    let cfg = ctx.store.guild_config(gid.get()).await?;
    if let Some(existing) = cfg.allow_role_id.map(Id::<RoleMarker>::new) {
        if snap.roles.contains_key(&existing) {
            return Ok((existing, false));
        }
    }
    let name = &ctx.config.allow_agent_role_name;
    if let Some(r) = snap.find_role_by_name(name) {
        // Heal config without creating a duplicate.
        ctx.store
            .update_guild_config(gid.get(), |c| c.allow_role_id = Some(r.get()))
            .await?;
        return Ok((r, false));
    }
    // Snapshot may be stale (role created seconds ago) — verify over HTTP
    // before creating to avoid duplicate same-name roles.
    if let Ok(fresh) = ctx.http.roles(gid).await?.model().await {
        if let Some(r) = fresh.iter().find(|r| r.name.eq_ignore_ascii_case(name)) {
            ctx.store
                .update_guild_config(gid.get(), |c| c.allow_role_id = Some(r.id.get()))
                .await?;
            return Ok((r.id, false));
        }
    }
    let role = ctx
        .http
        .create_role(gid)
        .name(name)
        .permissions(Permissions::empty())
        .mentionable(false)
        .reason("Umbraix Agent: access role for the server's shared Gemini key")
        .await?
        .model()
        .await?;
    ctx.store
        .update_guild_config(gid.get(), |c| c.allow_role_id = Some(role.id.get()))
        .await?;
    Ok((role.id, true))
}

pub async fn status(ctx: &Context, inv: Invocation) {
    let uid = inv.user_id.get();
    let mut lines = Vec::new();
    match ctx.store.get_key(KeyOwner::User(uid)).await {
        Ok(Some(k)) => lines.push(format!(
            "**Personal key:** `{}` · updated <t:{}:R>{}",
            k.hint,
            k.updated_at,
            k.last_used_at
                .map(|t| format!(" · last used <t:{t}:R>"))
                .unwrap_or_default()
        )),
        Ok(None) => lines.push(format!(
            "**Personal key:** not set — `{}byok-user`",
            ctx.prefix()
        )),
        Err(e) => {
            return inv
                .responder
                .error(ctx, &e.into(), None, &inv.request_id)
                .await
        }
    }

    if let Some(gid) = inv.guild_id {
        let cfg = match ctx.store.guild_config(gid.get()).await {
            Ok(c) => c,
            Err(e) => {
                return inv
                    .responder
                    .error(ctx, &e.into(), None, &inv.request_id)
                    .await
            }
        };
        match ctx.store.get_key(KeyOwner::Guild(gid.get())).await {
            Ok(Some(k)) => lines.push(format!(
                "**Server key:** `{}` · updated <t:{}:R>",
                k.hint, k.updated_at
            )),
            Ok(None) => lines.push("**Server key:** not set".into()),
            Err(e) => {
                return inv
                    .responder
                    .error(ctx, &e.into(), None, &inv.request_id)
                    .await
            }
        }
        lines.push(format!(
            "**Access role:** {}",
            cfg.allow_role_id
                .map(|r| format!("<@&{r}>"))
                .unwrap_or_else(|| "not set".into())
        ));
        lines.push(format!(
            "**Agent here:** {} · personal keys {}",
            if cfg.enabled { "enabled" } else { "disabled" },
            if cfg.allow_user_byok {
                "allowed"
            } else {
                "blocked"
            }
        ));
        let snapshot = GuildSnapshot::load(ctx, gid).await.ok();
        let is_guild_owner = snapshot
            .as_ref()
            .map(|s| s.owner_id == inv.user_id)
            .unwrap_or(false);
        let fresh_roles =
            access::fresh_member_roles(ctx, gid, inv.user_id, &inv.member_roles).await;
        let verdict = match access::resolve_with_snapshot(
            ctx,
            uid,
            Some((gid, is_guild_owner, &fresh_roles)),
            snapshot.as_ref(),
        )
        .await
        {
            Ok(Ok(g)) => format!(
                "✅ your requests here use the **{}** (`{}`)",
                g.source.label(),
                g.model
            ),
            Ok(Err(_)) => "❌ no access here yet".into(),
            Err(_) => "unknown".into(),
        };
        lines.push(format!("\n**Your access:** {verdict}"));
    } else if ctx.is_owner(uid) {
        lines.push("\n**Your access:** bot owner — operator key everywhere".into());
    }
    inv.responder
        .info(ctx, Tone::Info, "Key status", &lines.join("\n"), true)
        .await;
}

pub async fn remove(ctx: &Context, inv: Invocation, args: &str) {
    let scope = args.trim().to_lowercase();
    let result = match scope.as_str() {
        "user" | "me" | "personal" => {
            ctx.byok_sessions.remove(&inv.user_id.get());
            ctx.store
                .delete_key(KeyOwner::User(inv.user_id.get()))
                .await
                .map(|deleted| ("Personal key", deleted))
        }
        "server" | "guild" => {
            let Some(gid) = inv.guild_id else {
                inv.responder
                    .info(
                        ctx,
                        Tone::Warn,
                        "Server only",
                        "Run this inside the server whose key you want to delete.",
                        true,
                    )
                    .await;
                return;
            };
            let is_owner = ctx.is_owner(inv.user_id.get())
                || GuildSnapshot::load(ctx, gid)
                    .await
                    .map(|s| s.owner_id == inv.user_id)
                    .unwrap_or(false);
            if !is_owner {
                inv.responder
                    .info(
                        ctx,
                        Tone::Warn,
                        "Not allowed",
                        "Only the server owner can delete the server key.",
                        true,
                    )
                    .await;
                return;
            }
            ctx.store
                .delete_key(KeyOwner::Guild(gid.get()))
                .await
                .map(|deleted| ("Server key", deleted))
        }
        _ => {
            inv.responder
                .info(
                    ctx,
                    Tone::Info,
                    "Usage",
                    &format!(
                        "`{}byok-remove user` or `{}byok-remove server`",
                        ctx.prefix(),
                        ctx.prefix()
                    ),
                    true,
                )
                .await;
            return;
        }
    };
    match result {
        Ok((what, true)) => {
            tracing::info!(user = %inv.user_id, scope, "BYOK key deleted");
            inv.responder
                .info(ctx, Tone::Success, &format!("{what} deleted"), "The encrypted key was permanently removed. Consider also revoking it in Google AI Studio if it may have leaked.", true)
                .await
        }
        Ok((what, false)) => {
            inv.responder
                .info(
                    ctx,
                    Tone::Info,
                    "Nothing to delete",
                    &format!("No {} is stored.", what.to_lowercase()),
                    true,
                )
                .await
        }
        Err(e) => {
            inv.responder
                .error(ctx, &e.into(), None, &inv.request_id)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_roundtrip() {
        assert_eq!(parse_scope("user"), Some(ByokScope::User));
        let s = ByokScope::Server(Id::new(123));
        assert_eq!(parse_scope(&scope_suffix(s)), Some(s));
        assert_eq!(parse_scope("server:abc"), None);
        assert_eq!(parse_scope("server:0"), None);
    }

    #[test]
    fn sanitizes_keys() {
        assert!(sanitize_key("`AIzaSyA1234567890abcdefghijklmnopqrstu`").is_ok());
        assert!(sanitize_key("short").is_err());
        assert!(sanitize_key("AIza with spaces in it 1234567890").is_err());
    }
}
