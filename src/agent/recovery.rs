use std::sync::Arc;

use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, UserMarker},
    Id,
};

use crate::{
    access,
    agent::{is_resumable_error, run_agent_turn_resumable, ResumeInfo, TurnRequest},
    brand::Tone,
    concurrency::Metrics,
    context::Context,
    discord::reply::Responder,
    storage::PendingTurn,
    tools::ToolGuard,
};

/// Boot recovery: resume turns interrupted by a network drop / restart.
///
/// Called once (first `Ready`); runs in the background so gateway startup
/// never blocks. For each orphaned `pending_turns` row:
/// 1. Atomically claim it (no double-resume across shards) with an
///    attempt cap + age TTL — a poison request can never infinite-loop.
/// 2. Verify the guild/channel still exists and re-resolve access (keys or
///    roles may have changed while offline).
/// 3. Restore conversation history, post a "resuming…" notice, and re-drive
///    the turn with a re-grounding reminder (model MUST `list_*` first).
/// 4. `DELETE` the row on completion / terminal failure / expiry.
pub fn spawn_pending_recovery(ctx: &Context) {
    if !ctx.config.resume_enabled {
        return;
    }
    if ctx
        .recovery_started
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return; // already running (multiple shards emit Ready)
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        // Let the gateway cache + bot IDs settle before touching Discord.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if ctx.is_shutting_down() {
            return;
        }
        recover_pending_turns(&ctx).await;
    });
}

async fn recover_pending_turns(ctx: &Context) {
    let max_age = ctx.config.resume_max_age_secs;
    match ctx.store.prune_expired_pending(max_age).await {
        Ok(n) if n > 0 => tracing::info!(pruned = n, "Pruned expired pending turns"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "Failed to prune expired pending turns"),
    }
    let turns = match ctx.store.list_recoverable_turns(max_age, 10).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "Pending-turn recovery scan failed (gateway continues normally)");
            return;
        }
    };
    if turns.is_empty() {
        return;
    }
    tracing::info!(
        count = turns.len(),
        "Resuming interrupted turns after restart"
    );

    // Cap concurrent resumes so a boot with several orphans can't stampede
    // Gemini / Discord. One attempt per row per boot — never tight-loop.
    let sem = Arc::new(tokio::sync::Semaphore::new(2));
    let mut set = tokio::task::JoinSet::new();
    for turn in turns {
        let ctx = ctx.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            resume_one(&ctx, turn).await;
            Some(())
        });
    }
    while set.join_next().await.is_some() {}
    tracing::info!("Interrupted-turn recovery scan finished");
}

async fn resume_one(ctx: &Context, turn: PendingTurn) {
    // Budget check BEFORE claiming a user slot: over-budget rows are
    // deleted (never retried) so they can't loop across reboots.
    if turn.attempts >= turn.max_attempts.max(1) {
        tracing::warn!(request = %turn.request_id, attempts = turn.attempts, "Pending turn over budget, dropping");
        let _ = ctx.store.delete_pending(&turn.request_id).await;
        notify_channel(
            ctx,
            turn.channel_id,
            Tone::Warn,
            "Stale request dropped",
            &format!(
                "<@{}>, I was interrupted while working on \"{}\" but it already used all {} resume attempts, so I stopped instead of looping. Please send the request again.",
                turn.user_id,
                crate::utils::truncate_chars(&turn.prompt, 200),
                turn.max_attempts,
            ),
        )
        .await;
        return;
    }
    // One active turn per user: if the user already started something new
    // since the crash, leave this row for the next boot (don't steal slot).
    let user_slot = ctx.limiter.try_claim_user(turn.user_id);
    if user_slot.is_none() {
        tracing::info!(request = %turn.request_id, user = turn.user_id, "User busy, leaving pending turn for later");
        return;
    }
    let _user_slot = user_slot;

    let Some(claimed) = (match ctx.store.claim_pending_turn(&turn.request_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(request = %turn.request_id, error = %e, "Failed to claim pending turn");
            return;
        }
    }) else {
        return; // claimed by another shard / gone
    };
    if claimed.attempts > claimed.max_attempts.max(1) {
        let _ = ctx.store.delete_pending(&claimed.request_id).await;
        return;
    }

    let Some(guild_id) = Id::<GuildMarker>::new_checked(claimed.guild_id) else {
        let _ = ctx.store.delete_pending(&claimed.request_id).await;
        return;
    };
    let Some(channel_id) = Id::<ChannelMarker>::new_checked(claimed.channel_id) else {
        let _ = ctx.store.delete_pending(&claimed.request_id).await;
        return;
    };
    let Some(user_id) = Id::<UserMarker>::new_checked(claimed.user_id) else {
        let _ = ctx.store.delete_pending(&claimed.request_id).await;
        return;
    };

    // The channel/guild may be gone (deleted while offline) — verify over
    // HTTP, not cache. Gone targets are deleted, not retried.
    if ctx.http.channel(channel_id).await.is_err() {
        tracing::warn!(request = %claimed.request_id, channel = %channel_id, "Resume target channel gone, dropping turn");
        let _ = ctx.store.delete_pending(&claimed.request_id).await;
        return;
    }

    let snapshot = match access::GuildSnapshot::load(ctx, guild_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(request = %claimed.request_id, error = %e, "Guild gone during resume, dropping turn");
            let _ = ctx.store.delete_pending(&claimed.request_id).await;
            return;
        }
    };
    let is_guild_owner = snapshot.owner_id == user_id;
    let is_bot_owner = ctx.is_owner(claimed.user_id);
    let fresh_roles = access::fresh_member_roles(ctx, guild_id, user_id, &[]).await;
    let grant = match access::resolve_with_snapshot(
        ctx,
        claimed.user_id,
        Some((guild_id, is_guild_owner, &fresh_roles)),
        Some(&snapshot),
    )
    .await
    {
        Ok(Ok(g)) => g,
        Ok(Err(_denied)) => {
            tracing::info!(request = %claimed.request_id, "Access changed while offline, dropping resumed turn");
            let _ = ctx.store.delete_pending(&claimed.request_id).await;
            notify_channel(
                ctx,
                claimed.channel_id,
                Tone::Warn,
                "Couldn't auto-resume",
                &format!(
                    "<@{}>, I was interrupted while working on \"{}\" but your access changed while I was offline, so I stopped. Please send the request again if you still need it.",
                    claimed.user_id,
                    crate::utils::truncate_chars(&claimed.prompt, 200),
                ),
            )
            .await;
            return;
        }
        Err(e) => {
            tracing::warn!(request = %claimed.request_id, error = %e, "Access resolve failed during resume");
            let _ = ctx
                .store
                .park_pending_for_retry(
                    &claimed.request_id,
                    &claimed.history_json,
                    &claimed.tool_summary,
                    &e.to_string(),
                )
                .await;
            return;
        }
    };

    // Restore conversation history so the model sees pre-interruption tool
    // results (already scrubbed at checkpoint time).
    let mem_key = (channel_id.get(), user_id.get());
    if !claimed.history_json.trim().is_empty() && claimed.history_json.trim() != "[]" {
        match serde_json::from_str::<Vec<serde_json::Value>>(&claimed.history_json) {
            Ok(entries) if !entries.is_empty() => {
                let kept: Vec<_> = entries.into_iter().take(100).collect();
                ctx.memory.record(mem_key, kept).await;
            }
            _ => {}
        }
    }

    let responder = Responder::channel(channel_id, None);
    responder
        .info(
            ctx,
            Tone::Info,
            "Resuming interrupted work",
            &format!(
                "<@{}>, I was interrupted by a network issue / restart while working on \"{}\". \
                 I'll re-check the server state first so I don't duplicate anything, then finish the remaining work.",
                claimed.user_id,
                crate::utils::truncate_chars(&claimed.prompt, 300),
            ),
            false,
        )
        .await;

    let guard = ToolGuard {
        guild_id,
        authority: snapshot.authority(user_id, &fresh_roles, is_bot_owner),
        snapshot: std::sync::Arc::new(snapshot),
        bot_id: ctx.bot_user_id.get().copied(),
    };
    let req = TurnRequest {
        guild_id,
        channel_id,
        user_id,
        user_name: claimed.user_name.clone(),
        prompt: claimed.prompt.clone(),
        request_id: claimed.request_id.clone(),
        is_guild_owner,
        is_bot_owner,
    };
    let resume = ResumeInfo {
        attempt: claimed.attempts.max(1) as u32,
        original_request_id: claimed.request_id.clone(),
        tool_summary: claimed.tool_summary.clone(),
    };

    Metrics::inc(&ctx.metrics.turns_started);
    let result = run_agent_turn_resumable(
        ctx,
        &req,
        &grant,
        &guard,
        &responder,
        Some(claimed.request_id.clone()),
        Some(resume),
    )
    .await;

    match result {
        Ok(o) => {
            Metrics::inc(&ctx.metrics.turns_ok);
            tracing::info!(request = %claimed.request_id, tool_calls = o.tool_calls, "Resumed turn completed");
            if let Err(e) = ctx.store.delete_pending(&claimed.request_id).await {
                tracing::warn!(error = %e, request = %claimed.request_id, "Failed to delete completed pending turn");
            }
            let ctx2 = ctx.clone();
            tokio::spawn(async move {
                let _ = ctx2
                    .store
                    .record_usage("user", claimed.user_id, 1, o.tool_calls as i64, 0)
                    .await;
            });
        }
        Err(e) => {
            Metrics::inc(&ctx.metrics.turns_failed);
            tracing::warn!(request = %claimed.request_id, code = e.code(), error = %e, "Resumed turn failed");
            if is_resumable_error(&e) {
                // Park for the NEXT boot (one attempt per boot — no tight loop).
                let hist = ctx.memory.history(mem_key).await;
                let tail = hist[hist.len().saturating_sub(100)..].to_vec();
                let history_json =
                    serde_json::to_string(&tail).unwrap_or_else(|_| claimed.history_json.clone());
                if let Err(db) = ctx
                    .store
                    .park_pending_for_retry(
                        &claimed.request_id,
                        &history_json,
                        &claimed.tool_summary,
                        &e.to_string(),
                    )
                    .await
                {
                    tracing::warn!(error = %db, "Failed to park pending turn for retry");
                }
                responder
                    .error(ctx, &e, Some(grant.source), &claimed.request_id)
                    .await;
            } else {
                // Terminal: tell the user and delete so it never retries.
                responder
                    .error(ctx, &e, Some(grant.source), &claimed.request_id)
                    .await;
                if let Err(db) = ctx.store.delete_pending(&claimed.request_id).await {
                    tracing::warn!(error = %db, "Failed to delete terminal pending turn");
                }
            }
        }
    }
}

async fn notify_channel(ctx: &Context, channel_id: u64, tone: Tone, title: &str, body: &str) {
    let Some(ch) = Id::<ChannelMarker>::new_checked(channel_id) else {
        return;
    };
    Responder::channel(ch, None)
        .info(ctx, tone, title, body, false)
        .await;
}
