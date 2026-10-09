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
    // NEVER send a pasted API key to Gemini, store it in memory history, or
    // write it to audit logs. Prefix guild messages are already intercepted
    // by protect_leaked_key, but slash `/ai` bypasses that path — so check
    // here for both. Refuse without touching the prompt further.
    if crate::utils::contains_google_key(&prompt) {
        tracing::warn!(user = %inv.user_id, guild = ?inv.guild_id, "AI prompt contained an API key; refused");
        inv.responder
            .info(
                ctx,
                Tone::Error,
                "I can't accept API keys here",
                &format!(
                    "Your message looks like it contains a Gemini API key, so I stopped before sending anything to the AI.\n\n\
                    Anyone who saw it can use it — **revoke it now** at https://aistudio.google.com/apikey and create a new one.\n\
                    Then add keys only via `{}byok-user` (private form/DM) or `{}byok-server` (server owner). Never paste keys into chat or `/ai` prompts.",
                    ctx.prefix(),
                    ctx.prefix()
                ),
                true,
            )
            .await;
        return;
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
        inv.responder
            .info(
                ctx,
                crate::brand::Tone::Warn,
                "Server only",
                "AI turns run in servers — DMs are reserved for key setup (`byok-user`).",
                true,
            )
            .await;
        return;
    };
    let user = inv.user_id.get();
    let is_bot_owner = ctx.is_owner(user);

    // Ack slash interactions immediately (Discord requires <3s). Prefix
    // commands are a no-op. Doing this BEFORE slow snapshot/DB work prevents
    // "Unknown interaction" under load.
    if inv.is_slash {
        if let Err(e) = inv.responder.defer(ctx, false).await {
            tracing::warn!(error = %e, "Failed to defer interaction");
        }
    }

    // One active turn per user — including the bot owner. The old code let
    // the owner run unlimited parallel turns with the same (channel,user)
    // memory key, interleaving histories and burning quota.
    let slot = ctx.limiter.try_claim_user(user);
    if slot.is_none() {
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

    // Gateway payload roles can be empty/stale. Fetch authoritative roles
    // over HTTP for the access decision; fall back to payload on failure.
    // This fixes "I have the role but the bot denies me".
    let fresh_roles =
        access::fresh_member_roles(ctx, guild_id, inv.user_id, &inv.member_roles).await;

    let grant = match access::resolve_with_snapshot(
        ctx,
        user,
        Some((guild_id, is_guild_owner, &fresh_roles)),
        Some(&snapshot),
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

    // For prefix commands defer is a no-op; for slash it was already done
    // above (second call is harmless).
    if let Err(e) = inv.responder.defer(ctx, false).await {
        tracing::debug!(error = %e, "Defer after admission (already deferred)");
    }

    let guard = ToolGuard {
        guild_id,
        authority: snapshot.authority(inv.user_id, &fresh_roles, is_bot_owner),
        snapshot: snapshot.clone(),
        bot_id: ctx.bot_user_id.get().copied(),
    };
    // Crash-safe resume: persist the turn BEFORE running it. If the process
    // dies / network drops mid-turn, the row survives and boot recovery
    // resumes it once (with server-state re-grounding). Deleted on
    // completion / terminal failure so it can never loop.
    let (scrubbed_pending_prompt, _) = crate::utils::scrub_google_keys(&prompt);
    let pending = crate::storage::PendingTurn {
        request_id: inv.request_id.clone(),
        guild_id: guild_id.get(),
        channel_id: channel_id.get(),
        user_id: user,
        user_name: inv.user_name.clone(),
        prompt: scrubbed_pending_prompt,
        history_json: "[]".to_string(),
        tool_summary: String::new(),
        status: "pending".to_string(),
        attempts: 0,
        max_attempts: ctx.config.resume_max_attempts,
        created_at: crate::utils::now_secs(),
        updated_at: crate::utils::now_secs(),
        last_error: None,
    };
    if ctx.config.resume_enabled {
        if let Err(e) = ctx.store.create_pending_turn(&pending).await {
            tracing::debug!(error = %e, rid = %inv.request_id, "Failed to persist pending turn (best-effort)");
        }
    }
    let req = TurnRequest {
        guild_id,
        channel_id,
        user_id: inv.user_id,
        user_name: inv.user_name.clone(),
        prompt,
        request_id: inv.request_id.clone(),
        is_guild_owner,
        is_bot_owner,
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
    let pending_id = ctx.config.resume_enabled.then(|| inv.request_id.clone());
    let result = agent::run_agent_turn_resumable(
        ctx,
        &req,
        &grant,
        &guard,
        &inv.responder,
        pending_id.clone(),
        None,
    )
    .instrument(span.clone())
    .await;
    let ms = started.elapsed().as_millis() as u64;

    let (tool_calls, failed) = match &result {
        Ok(o) => {
            Metrics::inc(&ctx.metrics.turns_ok);
            tracing::info!(parent: &span, ms, steps = o.steps, tool_calls = o.tool_calls, tool_errors = o.tool_errors, "Turn completed");
            // Done — remove the resume row so a later reboot can't replay it.
            if let Some(pid) = pending_id.as_deref() {
                if let Err(e) = ctx.store.delete_pending(pid).await {
                    tracing::debug!(error = %e, request = pid, "Failed to delete pending turn after success");
                }
            }
            (o.tool_calls as i64, false)
        }
        Err(e) => {
            Metrics::inc(&ctx.metrics.turns_failed);
            tracing::warn!(parent: &span, ms, code = e.code(), error = %e, "Turn failed");
            inv.responder
                .error(ctx, e, Some(grant.source), &inv.request_id)
                .await;
            // Keep retryable failures for boot resume; delete terminal ones.
            if let Some(pid) = pending_id.as_deref() {
                if agent::is_resumable_error(e) {
                    if let Err(db) = ctx.store.mark_pending_error(pid, &e.to_string()).await {
                        tracing::debug!(error = %db, request = pid, "Failed to mark pending turn error");
                    }
                } else if let Err(db) = ctx.store.delete_pending(pid).await {
                    tracing::debug!(error = %db, request = pid, "Failed to delete terminal pending turn");
                }
            }
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
