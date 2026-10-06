use twilight_gateway::{EventTypeFlags, Intents, Shard, ShardId, StreamExt};

use crate::{context::Context, discord::events::handle_event};

/// Own the gateway connection: create the shard, filter to needed events,
/// and dispatch each one to [`handle_event`].
pub async fn run(ctx: Context) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let intents = Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT;
    let mut shard = Shard::new(ShardId::ONE, ctx.config.discord_token.clone(), intents);

    tracing::info!(
        "Bot initializing. Only Owner ID: {} can invoke '!ai <prompt>' (max {} steps)",
        ctx.config.owner_id,
        ctx.config.max_iterations
    );

    let event_filter = EventTypeFlags::MESSAGE_CREATE | EventTypeFlags::READY;

    while let Some(item) = shard.next_event(event_filter).await {
        let event = match item {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(?err, "Gateway frame warning");
                continue;
            }
        };
        // Spawn per event so a slow Gemini turn can't stall the gateway.
        // `handle_event` spawns its own task for message work; Ready is cheap.
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            handle_event(event, ctx_clone).await;
        });
    }

    Ok(())
}
