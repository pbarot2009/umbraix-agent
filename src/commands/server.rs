use std::str::FromStr;
use twilight_model::id::{
    marker::{ChannelMarker, RoleMarker},
    Id,
};

use super::Invocation;
use crate::{
    access::GuildSnapshot,
    brand::Tone,
    context::Context,
    storage::{GuildConfig, KeyOwner},
};

type ConfigUpdate = Box<dyn FnOnce(&mut GuildConfig) + Send>;

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" | "enable" | "enabled" | "1" => Some(true),
        "off" | "false" | "no" | "disable" | "disabled" | "0" => Some(false),
        _ => None,
    }
}

fn strip_mention(v: &str) -> &str {
    v.trim()
        .trim_start_matches("<@&")
        .trim_start_matches("<#")
        .trim_end_matches('>')
}

pub fn valid_model_name(v: &str) -> bool {
    // Accept optional `models/` prefix (Gemini API style) and standard IDs.
    // Reject path traversal, spaces, and empty segments.
    let bare = v.strip_prefix("models/").unwrap_or(v);
    if bare.is_empty() || bare.len() > 128 {
        return false;
    }
    if bare.contains("..") || bare.contains("//") || bare.contains(' ') {
        return false;
    }
    if bare.starts_with('/') || bare.ends_with('/') || bare.starts_with('.') {
        return false;
    }
    bare.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '/'))
}

async fn render(ctx: &Context, gid: u64, cfg: &GuildConfig) -> String {
    let server_key = match ctx.store.get_key(KeyOwner::Guild(gid)).await {
        Ok(Some(k)) => format!("`{}`", k.hint),
        Ok(None) => "not set".into(),
        Err(_) => "unavailable".into(),
    };
    let p = ctx.prefix();
    format!(
        "**enabled:** {}\n**user-byok:** {}\n**model:** `{}`{}\n**cooldown:** {}s{}\n**role:** {}\n**log-channel:** {}\n**server key:** {server_key}\n\nChange with `{p}config <setting> <value>` — see `{p}help config`.",
        if cfg.enabled { "on" } else { "off" },
        if cfg.allow_user_byok { "on" } else { "off" },
        cfg.model.as_deref().unwrap_or(&ctx.config.gemini_model),
        if cfg.model.is_none() { " (default)" } else { "" },
        cfg.cooldown_secs.unwrap_or(ctx.rate_limiter.default_cooldown().as_secs()),
        if cfg.cooldown_secs.is_none() { " (default)" } else { "" },
        cfg.allow_role_id.map(|r| format!("<@&{r}>")).unwrap_or_else(|| "not set".into()),
        cfg.log_channel_id.map(|c| format!("<#{c}>")).unwrap_or_else(|| "off".into()),
    )
}

pub async fn config(ctx: &Context, inv: Invocation, args: &str) {
    let Some(gid) = inv.guild_id else { return };
    let mut parts = args.trim().splitn(2, char::is_whitespace);
    let setting = parts.next().unwrap_or("").to_lowercase();
    let value = parts.next().unwrap_or("").trim().to_string();

    if setting.is_empty() || setting == "show" {
        return match ctx.store.guild_config(gid.get()).await {
            Ok(cfg) => {
                let body = render(ctx, gid.get(), &cfg).await;
                inv.responder
                    .info(ctx, Tone::Info, "Server settings", &body, true)
                    .await
            }
            Err(e) => {
                inv.responder
                    .error(ctx, &e.into(), None, &inv.request_id)
                    .await
            }
        };
    }

    let usage = |msg: &str| format!("{msg}\nSee `{}help config`.", ctx.prefix());
    let update: Result<ConfigUpdate, String> = match setting.as_str() {
        "enabled" => parse_bool(&value)
            .map(|b| Box::new(move |c: &mut GuildConfig| c.enabled = b) as ConfigUpdate)
            .ok_or_else(|| usage("Use `on` or `off`.")),
        "user-byok" | "userbyok" | "personal-keys" => parse_bool(&value)
            .map(|b| Box::new(move |c: &mut GuildConfig| c.allow_user_byok = b) as ConfigUpdate)
            .ok_or_else(|| usage("Use `on` or `off`.")),
        "model" => {
            if value.is_empty() || value.eq_ignore_ascii_case("default") {
                Ok(Box::new(|c: &mut GuildConfig| c.model = None))
            } else if valid_model_name(&value) {
                let v = value.clone();
                Ok(Box::new(move |c: &mut GuildConfig| c.model = Some(v)))
            } else {
                Err(usage("Model names look like `gemini-2.5-flash`."))
            }
        }
        "cooldown" => {
            if value.is_empty() || value.eq_ignore_ascii_case("default") {
                Ok(Box::new(|c: &mut GuildConfig| c.cooldown_secs = None))
            } else {
                match value.trim_end_matches('s').parse::<u64>() {
                    Ok(n) if (1..=3600).contains(&n) => Ok(Box::new(move |c: &mut GuildConfig| {
                        c.cooldown_secs = Some(n)
                    })),
                    _ => Err(usage("Cooldown must be 1-3600 seconds.")),
                }
            }
        }
        "role" => match Id::<RoleMarker>::from_str(strip_mention(&value)) {
            Ok(role) => match GuildSnapshot::load(ctx, gid).await {
                Ok(snap) if snap.roles.contains_key(&role) && role != gid.cast() => {
                    Ok(Box::new(move |c: &mut GuildConfig| {
                        c.allow_role_id = Some(role.get())
                    }))
                }
                Ok(_) => Err("That role isn't in this server.".into()),
                Err(e) => return inv.responder.error(ctx, &e, None, &inv.request_id).await,
            },
            Err(_) => match GuildSnapshot::load(ctx, gid)
                .await
                .ok()
                .and_then(|s| s.find_role_by_name(value.trim()))
            {
                Some(role) => Ok(Box::new(move |c: &mut GuildConfig| {
                    c.allow_role_id = Some(role.get())
                })),
                None => Err(usage(
                    "Mention a role (`@role`), give its ID, or its exact name.",
                )),
            },
        },
        "log-channel" | "logchannel" | "logs" => {
            if parse_bool(&value) == Some(false) || value.eq_ignore_ascii_case("none") {
                Ok(Box::new(|c: &mut GuildConfig| c.log_channel_id = None))
            } else {
                match Id::<ChannelMarker>::from_str(strip_mention(&value)) {
                    Ok(ch) => {
                        let in_guild = match ctx.cache.channel(ch) {
                            Some(c) => c.guild_id == Some(gid),
                            None => match ctx.http.channel(ch).await {
                                Ok(r) => r.model().await.ok().and_then(|c| c.guild_id) == Some(gid),
                                Err(_) => false,
                            },
                        };
                        if in_guild {
                            Ok(Box::new(move |c: &mut GuildConfig| {
                                c.log_channel_id = Some(ch.get())
                            }))
                        } else {
                            Err("That channel isn't in this server.".into())
                        }
                    }
                    Err(_) => Err(usage("Mention a channel (`#logs`) or use `off`.")),
                }
            }
        }
        "reset" => Ok(Box::new(|c: &mut GuildConfig| {
            let role = c.allow_role_id;
            *c = GuildConfig::default();
            c.allow_role_id = role;
        })),
        other => Err(usage(&format!("Unknown setting `{other}`."))),
    };

    match update {
        Err(msg) => {
            inv.responder
                .info(ctx, Tone::Warn, "Invalid setting", &msg, true)
                .await
        }
        Ok(f) => match ctx.store.update_guild_config(gid.get(), f).await {
            Ok(cfg) => {
                tracing::info!(guild = %gid, user = %inv.user_id, setting, value, "Guild config updated");
                let body = render(ctx, gid.get(), &cfg).await;
                inv.responder
                    .info(ctx, Tone::Success, "Settings updated", &body, true)
                    .await
            }
            Err(e) => {
                inv.responder
                    .error(ctx, &e.into(), None, &inv.request_id)
                    .await
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bools_and_models() {
        assert_eq!(parse_bool("ON"), Some(true));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
        assert!(valid_model_name("gemini-2.5-flash"));
        assert!(!valid_model_name("../../etc"));
        assert!(!valid_model_name("a b"));
        assert_eq!(strip_mention("<@&123>"), "123");
        assert_eq!(strip_mention("<#456>"), "456");
    }
}
