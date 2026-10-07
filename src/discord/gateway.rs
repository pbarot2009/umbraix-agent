use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use twilight_gateway::{
    create_recommended, CloseFrame, Config as ShardConfig, Event, EventTypeFlags, Intents, Shard,
    ShardId, StreamExt,
};

use crate::{brand, context::Context, discord::events::handle_event};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const INTENTS: Intents = Intents::GUILDS
    .union(Intents::GUILD_MESSAGES)
    .union(Intents::MESSAGE_CONTENT)
    .union(Intents::DIRECT_MESSAGES);

fn event_flags() -> EventTypeFlags {
    EventTypeFlags::READY
        | EventTypeFlags::MESSAGE_CREATE
        | EventTypeFlags::INTERACTION_CREATE
        | EventTypeFlags::GUILD_CREATE
        | EventTypeFlags::GUILD_UPDATE
        | EventTypeFlags::GUILD_DELETE
        | EventTypeFlags::ROLE_CREATE
        | EventTypeFlags::ROLE_UPDATE
        | EventTypeFlags::ROLE_DELETE
        | EventTypeFlags::CHANNEL_CREATE
        | EventTypeFlags::CHANNEL_UPDATE
        | EventTypeFlags::CHANNEL_DELETE
        | EventTypeFlags::THREAD_CREATE
        | EventTypeFlags::THREAD_UPDATE
        | EventTypeFlags::THREAD_DELETE
}

/// Start Discord's recommended number of shards, run until SIGINT/SIGTERM,
/// then shut down gracefully (close shards, drain in-flight turns, close DB).
pub async fn run(ctx: Context) -> Result<(), BoxError> {
    let config = ShardConfig::new(ctx.config.discord_token.clone(), INTENTS);
    let shards: Vec<Shard> = match create_recommended(&ctx.http, config.clone(), |_, b| b.build())
        .await
    {
        Ok(s) => s.collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Could not fetch recommended shard count; using 1 shard");
            vec![Shard::with_config(ShardId::ONE, config)]
        }
    };
    tracing::info!(
        shards = shards.len(),
        prefix = %ctx.config.command_prefix,
        max_concurrent = ctx.config.max_concurrent_turns,
        per_key = ctx.config.max_concurrent_per_key,
        "{} v{} starting",
        brand::NAME,
        brand::VERSION
    );

    let mut senders = Vec::with_capacity(shards.len());
    let mut tasks = tokio::task::JoinSet::new();
    for shard in shards {
        senders.push(shard.sender());
        tasks.spawn(runner(shard, ctx.clone()));
    }

    let all_stopped = tokio::select! {
        _ = shutdown_signal() => false,
        _ = async { while tasks.join_next().await.is_some() {} } => true,
    };
    if all_stopped {
        ctx.shutting_down.store(true, Ordering::Relaxed);
        ctx.store.close().await;
        return Err(
            "All gateway shards stopped (fatal close from Discord — see the error above).".into(),
        );
    }
    tracing::info!("Shutdown requested — closing shards and draining in-flight requests");
    ctx.shutting_down.store(true, Ordering::Relaxed);
    for s in &senders {
        let _ = s.close(CloseFrame::NORMAL);
    }
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;

    let drained = ctx.limiter.drain(Duration::from_secs(60)).await;
    if drained {
        tracing::info!("All in-flight requests finished");
    } else {
        tracing::warn!(
            active = ctx.limiter.active(),
            "Timed out waiting for in-flight requests"
        );
    }
    ctx.store.close().await;
    tracing::info!("{} stopped cleanly", brand::NAME);
    Ok(())
}

async fn runner(mut shard: Shard, ctx: Context) {
    let id = shard.id().number();
    let mut last_latency = Instant::now();
    while let Some(item) = shard.next_event(event_flags()).await {
        let event = match item {
            Ok(Event::GatewayClose(_)) if ctx.is_shutting_down() => break,
            Ok(Event::GatewayClose(frame)) => {
                match frame.as_ref().and_then(|f| fatal_close_hint(f.code)) {
                    Some(hint) => tracing::error!(shard = id, ?frame, hint, "Fatal gateway close"),
                    None => tracing::warn!(shard = id, ?frame, "Gateway closed; reconnecting"),
                }
                continue;
            }
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(shard = id, error = %err, "Gateway receive error");
                continue;
            }
        };
        ctx.cache.update(&event);
        if last_latency.elapsed() > Duration::from_secs(30) {
            if let Some(avg) = shard.latency().average() {
                ctx.shard_latency.insert(id, avg);
            }
            last_latency = Instant::now();
        }
        let ctx = ctx.clone();
        tokio::spawn(async move {
            handle_event(event, ctx).await;
        });
    }
    tracing::info!(shard = id, "Shard stopped");
}

fn fatal_close_hint(code: u16) -> Option<&'static str> {
    Some(match code {
        4004 => "DISCORD_TOKEN is invalid — reset it in the Discord Developer Portal",
        4010 | 4011 => "invalid shard configuration / sharding required",
        4012 => "invalid gateway API version",
        4013 => "invalid intents",
        4014 => "privileged intent not enabled — turn on MESSAGE CONTENT INTENT in Developer Portal → Bot",
        _ => return None,
    })
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
