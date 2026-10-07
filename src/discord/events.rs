use twilight_model::gateway::event::Event;

use crate::{
    agent,
    context::Context,
    discord::permissions::{extract_prompt, is_owner_command},
};

/// Route one gateway event. Message handling is spawned so the gateway
/// loop never blocks on Gemini round-trips.
pub async fn handle_event(event: Event, ctx: Context) {
    match event {
        Event::Ready(ready) => {
            tracing::info!(
                "Gateway ready! Logged in as: {}#{}",
                ready.user.name,
                ready.user.discriminator
            );
        }
        Event::MessageCreate(msg) => {
            if !is_owner_command(&msg, &ctx.config) {
                return;
            }
            let Some(prompt) = extract_prompt(&msg.content) else {
                // `!ai` with no prompt: nudge instead of silent ignore.
                if msg.content.trim() == "!ai" || msg.content.trim_start().starts_with("!ai ") {
                    let ctx_clone = ctx.clone();
                    let channel = msg.channel_id;
                    tokio::spawn(async move {
                        let _ = ctx_clone
                            .http
                            .create_message(channel)
                            .content("Usage: `!ai <request>` · `!ai help` · `!ai clear` (clears this channel's history)")
                            .await;
                    });
                }
                return;
            };

            // Built-in control commands (no model call, no rate-limit burn).
            let lowered = prompt.trim().to_lowercase();
            if lowered == "help" || prompt.trim() == "?" {
                let ctx_clone = ctx.clone();
                let channel = msg.channel_id;
                let steps = ctx.config.max_iterations;
                tokio::spawn(async move {
                    let _ = ctx_clone
                        .http
                        .create_message(channel)
                        .content(&format!(
                            "**Discord admin agent** (up to {steps} steps/request)\n\
                            `!ai <request>` — e.g. `!ai list channels`, `!ai timeout @user 10m spam`\n\
                            `!ai clear` — forget this channel's conversation history\n\
                            Grounding is automatic: I resolve names via list_channels/list_roles/search_members before acting.\n\
                            Destructive actions only run when you explicitly ask."
                        ))
                        .await;
                });
                return;
            }
            if lowered == "clear" || lowered == "reset" || lowered == "forget" {
                let ctx_clone = ctx.clone();
                let channel = msg.channel_id;
                let channel_u64 = channel.get();
                tokio::spawn(async move {
                    ctx_clone.memory.clear(channel_u64).await;
                    let _ = ctx_clone
                        .http
                        .create_message(channel)
                        .content("Cleared this channel's conversation history.")
                        .await;
                });
                return;
            }

            let user_id = msg.author.id.get();
            if let Err(cooldown) = ctx.rate_limiter.check(user_id).await {
                tracing::debug!(user_id, "Rate-limited !ai request");
                let ctx_clone = ctx.clone();
                let msg_clone = (*msg).clone();
                tokio::spawn(async move {
                    let _ = ctx_clone
                        .http
                        .create_message(msg_clone.channel_id)
                        .content(&format!(
                            "Slow down — try again in {}s.",
                            cooldown.retry_after_secs
                        ))
                        .await;
                });
                return;
            }

            let ctx_clone = ctx.clone();
            let msg_obj = (*msg).clone();
            tokio::spawn(async move {
                if let Err(e) = agent::run_agent_turn(&msg_obj, prompt, &ctx_clone).await {
                    tracing::error!(?e, "Error executing agent turn");
                    // Surface a truncated real error instead of a generic
                    // "something went wrong" so the owner can self-diagnose
                    // (bad args, missing perms, model outage) without logs.
                    let mut detail = e.to_string();
                    if detail.len() > 400 {
                        detail.truncate(400);
                        detail.push('…');
                    }
                    let _ = ctx_clone
                        .http
                        .create_message(msg_obj.channel_id)
                        .content(&format!("Something went wrong: {detail}"))
                        .await;
                }
            });
        }
        _ => {}
    }
}
