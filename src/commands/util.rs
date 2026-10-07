use std::time::Instant;

use super::{find, Access, Category, Invocation, COMMANDS};
use crate::{
    brand::{self, Tone},
    concurrency::Metrics,
    context::Context,
    utils::format_duration,
};

pub async fn help(ctx: &Context, inv: Invocation, args: &str) {
    let p = ctx.prefix();
    let query = args.trim().trim_start_matches(p).trim_start_matches('/');
    if !query.is_empty() {
        let Some(spec) = find(query) else {
            inv.responder
                .info(
                    ctx,
                    Tone::Warn,
                    "Unknown command",
                    &format!("No command named `{query}`. Try `{p}help`."),
                    true,
                )
                .await;
            return;
        };
        let aliases = if spec.aliases.is_empty() {
            String::new()
        } else {
            format!(
                "\n**Aliases:** {}",
                spec.aliases
                    .iter()
                    .map(|a| format!("`{p}{a}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let who = match spec.access {
            Access::Everyone => "everyone",
            Access::GuildOwner => "server owner",
            Access::BotOwner => "bot owner",
        };
        let body = format!(
            "**Usage:** `{p}{} {}` · `/{}`\n**Who:** {who}{}{aliases}\n\n{}",
            spec.name,
            spec.usage,
            spec.name,
            if spec.guild_only {
                " · servers only"
            } else {
                ""
            },
            spec.details
        );
        inv.responder
            .info(ctx, Tone::Info, &format!("{p}{}", spec.name), &body, true)
            .await;
        return;
    }

    let is_owner = ctx.is_owner(inv.user_id.get());
    let mut sections = Vec::new();
    for cat in Category::ALL {
        let lines: Vec<String> = COMMANDS
            .iter()
            .filter(|c| c.category == cat && (c.access != Access::BotOwner || is_owner))
            .map(|c| {
                let tag = if c.access == Access::GuildOwner {
                    " *(server owner)*"
                } else {
                    ""
                };
                let usage = if c.usage.is_empty() {
                    String::new()
                } else {
                    format!(" {}", c.usage)
                };
                format!("`{p}{}{usage}` — {}{tag}", c.name, c.summary)
            })
            .collect();
        if !lines.is_empty() {
            sections.push(format!("**{}**\n{}", cat.title(), lines.join("\n")));
        }
    }
    let body = format!(
        "{}\nEvery command also works as a slash command (`/help`, `/ai`, …).\n\n{}\n\n\
         **How access works:** the bot owner always has access · anyone with a personal key (`{p}byok-user`) can use me in any server · \
         the server owner and members with the access role share the server key (`{p}byok-server`). \
         The agent always acts within **your own** Discord permissions.",
        brand::TAGLINE,
        sections.join("\n\n")
    );
    inv.responder
        .info(
            ctx,
            Tone::Primary,
            &format!("{} — help", brand::NAME),
            &body,
            true,
        )
        .await;
}

pub async fn clear(ctx: &Context, inv: Invocation) {
    let Some(ch) = inv.channel_id else { return };
    ctx.memory.clear((ch.get(), inv.user_id.get())).await;
    inv.responder
        .info(
            ctx,
            Tone::Success,
            "Conversation cleared",
            "I've forgotten our conversation in this channel.",
            true,
        )
        .await;
}

pub async fn about(ctx: &Context, inv: Invocation) {
    let guilds = ctx.cache.stats().guilds();
    let tool_count = crate::tools::tool_names().len();
    let body = format!(
        "{}\n\n**Version:** {}\n**Uptime:** {}\n**Servers:** {guilds}\n**Model (default):** `{}`\n\n\
         • Bring your own Gemini key — personal or per-server, stored encrypted\n\
         • {tool_count} admin tools, always limited to your Discord permissions\n\
         • Built in Rust for high concurrency\n\nStart with `{}help`.",
        brand::TAGLINE,
        brand::VERSION,
        format_duration(ctx.started_at.elapsed().as_secs()),
        ctx.config.gemini_model,
        ctx.prefix()
    );
    inv.responder
        .info(ctx, Tone::Primary, brand::NAME, &body, true)
        .await;
}

pub async fn ping(ctx: &Context, inv: Invocation) {
    let started = Instant::now();
    let api = match ctx.http.current_user().await {
        Ok(_) => format!("{} ms", started.elapsed().as_millis()),
        Err(_) => "error".into(),
    };
    let gateway = {
        let vals: Vec<u128> = ctx
            .shard_latency
            .iter()
            .map(|e| e.value().as_millis())
            .collect();
        if vals.is_empty() {
            "measuring…".to_string()
        } else {
            format!(
                "{} ms avg ({} shard{})",
                vals.iter().sum::<u128>() / vals.len() as u128,
                vals.len(),
                if vals.len() == 1 { "" } else { "s" }
            )
        }
    };
    let db = match ctx.store.ping().await {
        Ok(()) => "ok",
        Err(_) => "unavailable",
    };
    let body = format!(
        "**Gateway:** {gateway}\n**REST API:** {api}\n**Database:** {db}\n**Load:** {}/{} active · {} queued",
        ctx.limiter.active(),
        ctx.limiter.capacity(),
        ctx.limiter.queued()
    );
    inv.responder
        .info(ctx, Tone::Info, "Pong", &body, true)
        .await;
}

pub async fn usage(ctx: &Context, inv: Invocation) {
    let uid = inv.user_id.get();
    let mut lines = Vec::new();
    match (
        ctx.store.usage("user", uid, 1).await,
        ctx.store.usage("user", uid, 30).await,
    ) {
        (Ok(d), Ok(m)) => lines.push(format!(
            "**You** — today: {} requests, {} tool calls · 30 days: {} requests, {} tool calls",
            d.requests, d.tool_calls, m.requests, m.tool_calls
        )),
        (Err(e), _) | (_, Err(e)) => {
            return inv
                .responder
                .error(ctx, &e.into(), None, &inv.request_id)
                .await
        }
    }
    if let Some(gid) = inv.guild_id {
        if let (Ok(d), Ok(m)) = (
            ctx.store.usage("guild", gid.get(), 1).await,
            ctx.store.usage("guild", gid.get(), 30).await,
        ) {
            lines.push(format!(
                "**Server key** — today: {} requests, {} tool calls · 30 days: {} requests, {} tool calls",
                d.requests, d.tool_calls, m.requests, m.tool_calls
            ));
        }
    }
    lines.push(
        "\nGemini quotas are set per Google project — check yours in Google AI Studio.".into(),
    );
    inv.responder
        .info(ctx, Tone::Info, "Usage", &lines.join("\n"), true)
        .await;
}

pub async fn stats(ctx: &Context, inv: Invocation) {
    let m = &ctx.metrics;
    let g = &ctx.gemini.stats;
    let counts = ctx.store.counts().await.unwrap_or_default();
    let body = format!(
        "**Uptime:** {}\n**Servers:** {} · **shards:** {}\n**Turns:** {} started · {} ok · {} failed\n\
         **Load:** {}/{} active · {} queued now · {} queued total · {} rejected\n\
         **Tools:** {} calls · {} blocked by permissions\n**Commands:** {}\n\
         **Gemini:** {} HTTP calls · {} retries · {} failures\n\
         **Stored keys:** {} personal · {} server · {} configured servers\n**Conversations in memory:** {}",
        format_duration(ctx.started_at.elapsed().as_secs()),
        ctx.cache.stats().guilds(),
        ctx.shard_latency.len(),
        Metrics::get(&m.turns_started),
        Metrics::get(&m.turns_ok),
        Metrics::get(&m.turns_failed),
        ctx.limiter.active(),
        ctx.limiter.capacity(),
        ctx.limiter.queued(),
        Metrics::get(&m.queued_total),
        Metrics::get(&m.rejected_busy),
        Metrics::get(&m.tool_calls),
        Metrics::get(&m.tool_denied),
        Metrics::get(&m.commands),
        Metrics::get(&g.requests),
        Metrics::get(&g.retries),
        Metrics::get(&g.failures),
        counts.user_keys,
        counts.guild_keys,
        counts.guild_configs,
        ctx.memory.conversations().await,
    );
    inv.responder
        .info(ctx, Tone::Info, "Runtime stats", &body, true)
        .await;
}
