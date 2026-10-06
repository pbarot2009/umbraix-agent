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
                return;
            };

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
                    let _ = ctx_clone
                        .http
                        .create_message(msg_obj.channel_id)
                        .content("Something went wrong while talking to the model.")
                        .await;
                }
            });
        }
        _ => {}
    }
}
