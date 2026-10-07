use twilight_gateway::{EventTypeFlags, Intents, Shard, ShardId, StreamExt};

use crate::{context::Context, discord::events::handle_event};

/// Own the gateway connection with reconnect backoff.
///
/// The old implementation returned `Ok(())` when the shard stream ended,
/// making systemd/docker think the bot exited cleanly. Now we reconnect
/// with exponential backoff so transient Discord outages don't kill the bot.
pub async fn run(ctx: Context) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let intents = Intents::GUILD_MESSAGES | Intents::MESSAGE_CONTENT | Intents::GUILDS;
    let event_filter = EventTypeFlags::MESSAGE_CREATE | EventTypeFlags::READY;

    tracing::info!(
        "Bot initializing. Only Owner ID: {} can invoke '!ai <prompt>' (max {} steps, turn timeout {}s)",
        ctx.config.owner_id,
        ctx.config.max_iterations,
        ctx.config.turn_timeout_secs,
    );

    let mut backoff_secs: u64 = 1;
    loop {
        let mut shard = Shard::new(ShardId::ONE, ctx.config.discord_token.clone(), intents);

        while let Some(item) = shard.next_event(event_filter).await {
            // Successful event resets reconnect backoff.
            backoff_secs = 1;
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

        tracing::warn!(
            backoff_secs,
            "Gateway stream ended; reconnecting with backoff"
        );
        tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(300);
    }
}
