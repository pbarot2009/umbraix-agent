use std::sync::atomic::Ordering;
use twilight_model::{channel::Message, gateway::event::Event};

use crate::{
    brand::{self, Tone},
    commands::{self, byok, Invocation},
    context::Context,
    discord::{interactions, reply::Responder},
    utils,
};

/// Route one gateway event. Called on its own task per event, so slow
/// work here never blocks the shard.
pub async fn handle_event(event: Event, ctx: Context) {
    match event {
        Event::Ready(ready) => {
            let _ = ctx.app_id.set(ready.application.id);
            let _ = ctx.bot_user_id.set(ready.user.id);
            tracing::info!(
                user = %ready.user.name,
                id = %ready.user.id,
                guilds = ready.guilds.len(),
                shard = ?ready.shard,
                "{} v{} connected",
                brand::NAME,
                brand::VERSION
            );
            if ctx.config.register_slash_commands
                && !ctx.commands_registered.swap(true, Ordering::Relaxed)
            {
                interactions::register_commands(&ctx).await;
            }
        }
        Event::GuildCreate(g) => {
            tracing::debug!(guild = %g.id(), "Guild available");
        }
        Event::GuildDelete(g) => {
            if g.unavailable.is_none() || g.unavailable == Some(false) {
                tracing::info!(guild = %g.id, "Removed from guild; purging its stored key and settings");
                if let Err(e) = ctx.store.purge_guild(g.id.get()).await {
                    tracing::error!(guild = %g.id, error = %e, "Failed to purge guild data");
                }
            } else {
                tracing::warn!(guild = %g.id, "Guild became unavailable (outage)");
            }
        }
        Event::MessageCreate(msg) => on_message(&ctx, &msg.0).await,
        Event::InteractionCreate(i) => interactions::handle(&ctx, i.0).await,
        _ => {}
    }
}

async fn on_message(ctx: &Context, msg: &Message) {
    if msg.author.bot {
        return;
    }

    if msg.guild_id.is_some() && utils::contains_google_key(&msg.content) {
        protect_leaked_key(ctx, msg).await;
        return;
    }

    if msg.guild_id.is_none() && byok::handle_dm_text(ctx, msg).await {
        return;
    }

    let Some((name, args)) = commands::parse_prefixed(&msg.content, ctx.prefix()) else {
        if msg.guild_id.is_none() && utils::contains_google_key(&msg.content) {
            let r = Responder::channel(msg.channel_id, Some(msg.id));
            r.info(
                ctx,
                Tone::Info,
                "No key setup in progress",
                &format!(
                    "Run `{}byok-user` first, then send the key (or use the secure button).",
                    ctx.prefix()
                ),
                false,
            )
            .await;
        }
        return;
    };
    let Some(spec) = commands::find(&name) else {
        return;
    };
    let inv = Invocation {
        user_id: msg.author.id,
        user_name: msg.author.name.clone(),
        guild_id: msg.guild_id,
        channel_id: Some(msg.channel_id),
        member_roles: msg
            .member
            .as_ref()
            .map(|m| m.roles.clone())
            .unwrap_or_default(),
        request_id: utils::short_id(8),
        responder: Responder::channel(msg.channel_id, Some(msg.id)),
        is_slash: false,
    };
    commands::dispatch(ctx, inv, spec, args.to_string()).await;
}

/// Someone pasted an API key in a server channel: delete it and warn.
async fn protect_leaked_key(ctx: &Context, msg: &Message) {
    let deleted = ctx
        .http
        .delete_message(msg.channel_id, msg.id)
        .await
        .is_ok();
    tracing::warn!(user = %msg.author.id, channel = %msg.channel_id, deleted, "API key posted in a public channel");
    let body = format!(
        "<@{}> {}\n\nAnyone who saw it can use it — **revoke it now** at https://aistudio.google.com/apikey and create a new one. \
         To add a key safely use `/byok-user` (private form) or `{}byok-user` (DM).",
        msg.author.id,
        if deleted {
            "I deleted your message because it contained an API key."
        } else {
            "your message contains an API key and I couldn't delete it (I need **Manage Messages**)."
        },
        ctx.prefix()
    );
    let _ = ctx
        .http
        .create_message(msg.channel_id)
        .embeds(&[brand::embed(Tone::Warn, "Secret detected", &body)])
        .await;
}
