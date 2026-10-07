use std::sync::Arc;
use tracing::Instrument;

use super::{find, util, Invocation};
use crate::{
    access::{self, GuildSnapshot, KeySource},
    agent::{self, TurnRequest},
    brand::Tone,
    concurrency::{Admission, Metrics},
    context::Context,
    storage::KeyOwner,
    tools::ToolGuard,
};

pub async fn run(ctx: &Context, inv: Invocation, args: String) {
    let prompt = args.trim().to_string();
    let lowered = prompt.to_lowercase();
    if prompt.is_empty() {
        let spec = find("ai").expect("ai command registered");
        inv.responder
            .info(
                ctx,
                Tone::Info,
                "Usage",
                &format!("`{}ai {}`\n\n{}", ctx.prefix(), spec.usage, spec.details),
                true,
            )
            .await;
        return;
    }
    if lowered == "help" || lowered == "?" {
        return util::help(ctx, inv, "").await;
    }
    if matches!(lowered.as_str(), "clear" | "reset" | "forget") {
        return util::clear(ctx, inv).await;
    }
    if ctx.is_shutting_down() {
        inv.responder
            .info(
                ctx,
                Tone::Warn,
                "Restarting",
                "Umbraix is restarting — please try again in a minute.",
                true,
            )
            .await;
        return;
    }
    let (Some(guild_id), Some(channel_id)) = (inv.guild_id, inv.channel_id) else {
        return;
    };
    let user = inv.user_id.get();

    let slot = ctx.limiter.try_claim_user(user);
    if slot.is_none() && !ctx.is_owner(user) {
        inv.responder
            .info(
                ctx,
                Tone::Warn,
                "Already working on it",
                "You already have a request running. Wait for it to finish, then try again.",
                true,
            )
            .await;
        return;
    }
    let _slot = slot;

    let snapshot = match GuildSnapshot::load(ctx, guild_id).await {
        Ok(s) => Arc::new(s),
        Err(e) => return inv.responder.error(ctx, &e, None, &inv.request_id).await,
    };
    let is_guild_owner = snapshot.owner_id == inv.user_id;

    let grant = match access::resolve(
        ctx,
        user,
        Some((guild_id, is_guild_owner, &inv.member_roles)),
    )
    .await
    {
        Ok(Ok(g)) => g,
        Ok(Err(denied)) => {
            tracing::info!(user, guild = %guild_id, ?denied, "AI request denied (no key access)");
            let (title, body) = denied.render(ctx.prefix(), &ctx.config.allow_agent_role_name);
            inv.responder
                .info(ctx, Tone::Warn, &title, &body, true)
                .await;
            return;
        }
        Err(e) => return inv.responder.error(ctx, &e, None, &inv.request_id).await,
    };

    if grant.source != KeySource::Owner {
        if let Err(cd) = ctx.rate_limiter.check(user, grant.cooldown) {
            inv.responder
                .info(
                    ctx,
                    Tone::Warn,
                    "Slow down",
                    &format!("Try again in {}s.", cd.retry_after_secs),
                    true,
                )
                .await;
            return;
        }
    }

    let _permit = match ctx.limiter.try_admit() {
        Admission::Immediate(p) => p,
        Admission::Queued => {
            Metrics::inc(&ctx.metrics.queued_total);
            let ahead = ctx.limiter.queued() + 1;
            tracing::info!(user, ahead, "Turn queued (at capacity)");
            inv.responder
                .info(
                    ctx,
                    Tone::Info,
                    "Queued",
                    &format!("Umbraix is busy right now — you're in line (~{ahead} ahead). I'll start automatically."),
                    false,
                )
                .await;
            match ctx.limiter.wait_admit().await {
                Some(p) => p,
                None => {
                    Metrics::inc(&ctx.metrics.rejected_busy);
                    inv.responder
                        .info(
                            ctx,
                            Tone::Error,
                            "At capacity",
                            "Too many requests right now. Please try again in a minute.",
                            true,
                        )
                        .await;
                    return;
                }
            }
        }
    };

    if let Err(e) = inv.responder.defer(ctx, false).await {
        tracing::warn!(error = %e, "Failed to defer interaction");
    }

    let guard = ToolGuard {
        guild_id,
        authority: snapshot.authority(inv.user_id, &inv.member_roles, ctx.is_owner(user)),
        snapshot: snapshot.clone(),
    };
    let req = TurnRequest {
        guild_id,
        channel_id,
        user_id: inv.user_id,
        user_name: inv.user_name.clone(),
        prompt,
        request_id: inv.request_id.clone(),
    };

    let span = tracing::info_span!(
        "turn",
        rid = %inv.request_id,
        guild = %guild_id,
        channel = %channel_id,
        user,
        key = grant.source.tag(),
        model = %grant.model,
    );
    Metrics::inc(&ctx.metrics.turns_started);
    let started = std::time::Instant::now();
    let result = agent::run_agent_turn(ctx, &req, &grant, &guard, &inv.responder)
        .instrument(span.clone())
        .await;
    let ms = started.elapsed().as_millis() as u64;

    let (tool_calls, failed) = match &result {
        Ok(o) => {
            Metrics::inc(&ctx.metrics.turns_ok);
            tracing::info!(parent: &span, ms, steps = o.steps, tool_calls = o.tool_calls, tool_errors = o.tool_errors, "Turn completed");
            (o.tool_calls as i64, false)
        }
        Err(e) => {
            Metrics::inc(&ctx.metrics.turns_failed);
            tracing::warn!(parent: &span, ms, code = e.code(), error = %e, "Turn failed");
            inv.responder
                .error(ctx, e, Some(grant.source), &inv.request_id)
                .await;
            (0, true)
        }
    };

    let ctx2 = ctx.clone();
    let key_owner = grant.key_owner;
    tokio::spawn(async move {
        let errors = failed as i64;
        if let Err(e) = ctx2
            .store
            .record_usage("user", user, 1, tool_calls, errors)
            .await
        {
            tracing::warn!(error = %e, "Failed to record usage");
        }
        if let Some(KeyOwner::Guild(g)) = key_owner {
            let _ = ctx2
                .store
                .record_usage("guild", g, 1, tool_calls, errors)
                .await;
        }
        if let Some(owner) = key_owner {
            let _ = ctx2.store.touch_key(owner).await;
        }
    });
}
