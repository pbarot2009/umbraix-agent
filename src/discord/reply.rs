use tokio::sync::Mutex;
use twilight_model::{
    channel::message::{AllowedMentions, Component, Embed, MessageFlags},
    http::interaction::{InteractionResponse, InteractionResponseType},
    id::{
        marker::{ApplicationMarker, ChannelMarker, InteractionMarker, MessageMarker, UserMarker},
        Id,
    },
};
use twilight_util::builder::InteractionResponseDataBuilder;

use crate::{
    access::KeySource,
    brand::{self, Tone},
    context::Context,
    error::BotError,
    utils,
};

/// No pings from bot output unless explicitly intended — model text can
/// never mass-mention. Built explicitly so an upstream default change can't
/// re-enable pings.
pub fn no_mentions() -> AllowedMentions {
    AllowedMentions {
        parse: vec![],
        users: vec![],
        roles: vec![],
        replied_user: false,
    }
}

#[derive(Debug, Clone)]
enum Target {
    Channel {
        channel_id: Id<ChannelMarker>,
        reply_to: Option<Id<MessageMarker>>,
    },
    Interaction {
        app_id: Id<ApplicationMarker>,
        id: Id<InteractionMarker>,
        token: String,
        channel_id: Option<Id<ChannelMarker>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Fresh,
    Deferred,
    Responded,
}

/// One reply surface for both prefix commands (channel messages) and slash
/// commands/components (interaction responses + followups).
pub struct Responder {
    target: Target,
    state: Mutex<State>,
}

impl Responder {
    pub fn channel(channel_id: Id<ChannelMarker>, reply_to: Option<Id<MessageMarker>>) -> Self {
        Self {
            target: Target::Channel {
                channel_id,
                reply_to,
            },
            state: Mutex::new(State::Fresh),
        }
    }

    pub fn interaction(
        app_id: Id<ApplicationMarker>,
        id: Id<InteractionMarker>,
        token: String,
        channel_id: Option<Id<ChannelMarker>>,
    ) -> Self {
        Self {
            target: Target::Interaction {
                app_id,
                id,
                token,
                channel_id,
            },
            state: Mutex::new(State::Fresh),
        }
    }

    pub fn is_interaction(&self) -> bool {
        matches!(self.target, Target::Interaction { .. })
    }

    pub fn channel_id(&self) -> Option<Id<ChannelMarker>> {
        match &self.target {
            Target::Channel { channel_id, .. } => Some(*channel_id),
            Target::Interaction { channel_id, .. } => *channel_id,
        }
    }

    /// Acknowledge an interaction within Discord's 3s window ("thinking…").
    /// No-op for channel targets.
    pub async fn defer(&self, ctx: &Context, ephemeral: bool) -> Result<(), BotError> {
        let Target::Interaction {
            app_id, id, token, ..
        } = &self.target
        else {
            return Ok(());
        };
        let mut state = self.state.lock().await;
        if *state != State::Fresh {
            return Ok(());
        }
        let mut data = InteractionResponseDataBuilder::new();
        if ephemeral {
            data = data.flags(MessageFlags::EPHEMERAL);
        }
        let resp = InteractionResponse {
            kind: InteractionResponseType::DeferredChannelMessageWithSource,
            data: Some(data.build()),
        };
        ctx.http
            .interaction(*app_id)
            .create_response(*id, token, &resp)
            .await?;
        *state = State::Deferred;
        Ok(())
    }

    /// Open a modal (interaction targets only, must be the first response).
    pub async fn modal(
        &self,
        ctx: &Context,
        custom_id: String,
        title: &str,
        components: Vec<Component>,
    ) -> Result<(), BotError> {
        let Target::Interaction {
            app_id, id, token, ..
        } = &self.target
        else {
            return Err(BotError::Internal("modal requires an interaction".into()));
        };
        let mut state = self.state.lock().await;
        if *state != State::Fresh {
            return Err(BotError::Internal(
                "interaction already acknowledged".into(),
            ));
        }
        let resp = InteractionResponse {
            kind: InteractionResponseType::Modal,
            data: Some(
                InteractionResponseDataBuilder::new()
                    .custom_id(custom_id)
                    .title(title)
                    .components(components)
                    .build(),
            ),
        };
        ctx.http
            .interaction(*app_id)
            .create_response(*id, token, &resp)
            .await?;
        *state = State::Responded;
        Ok(())
    }

    pub async fn embed(
        &self,
        ctx: &Context,
        embed: Embed,
        ephemeral: bool,
    ) -> Result<(), BotError> {
        self.send(ctx, vec![embed], Vec::new(), ephemeral).await
    }

    pub async fn send(
        &self,
        ctx: &Context,
        embeds: Vec<Embed>,
        components: Vec<Component>,
        ephemeral: bool,
    ) -> Result<(), BotError> {
        let mentions = no_mentions();
        match &self.target {
            Target::Channel {
                channel_id,
                reply_to,
            } => {
                let mut req = ctx
                    .http
                    .create_message(*channel_id)
                    .embeds(&embeds)
                    .components(&components)
                    .allowed_mentions(Some(&mentions));
                if let Some(r) = reply_to {
                    req = req.reply(*r).fail_if_not_exists(false);
                }
                req.await?;
                Ok(())
            }
            Target::Interaction {
                app_id,
                id,
                token,
                channel_id,
            } => {
                let mut state = self.state.lock().await;
                let client = ctx.http.interaction(*app_id);
                let result: Result<(), BotError> = match *state {
                    State::Fresh => {
                        let mut data = InteractionResponseDataBuilder::new()
                            .embeds(embeds.clone())
                            .components(components.clone())
                            .allowed_mentions(mentions.clone());
                        if ephemeral {
                            data = data.flags(MessageFlags::EPHEMERAL);
                        }
                        let resp = InteractionResponse {
                            kind: InteractionResponseType::ChannelMessageWithSource,
                            data: Some(data.build()),
                        };
                        client
                            .create_response(*id, token, &resp)
                            .await
                            .map(|_| ())
                            .map_err(Into::into)
                    }
                    State::Deferred => client
                        .update_response(token)
                        .embeds(Some(&embeds))
                        .components(Some(&components))
                        .allowed_mentions(Some(&mentions))
                        .await
                        .map(|_| ())
                        .map_err(Into::into),
                    State::Responded => {
                        let mut req = client
                            .create_followup(token)
                            .embeds(&embeds)
                            .components(&components)
                            .allowed_mentions(Some(&mentions));
                        if ephemeral {
                            req = req.flags(MessageFlags::EPHEMERAL);
                        }
                        req.await.map(|_| ()).map_err(Into::into)
                    }
                };
                match result {
                    Ok(()) => {
                        *state = State::Responded;
                        Ok(())
                    }
                    Err(e) => {
                        // Interaction tokens expire after 15 minutes; long
                        // agent runs fall back to a plain channel message.
                        // Fresh-state failures are usually "Unknown interaction"
                        // (3s window expired) — also fall back when the error
                        // looks like expiry, otherwise surface it.
                        let err_str = e.to_string();
                        let expired = err_str.contains("10062")
                            || err_str.to_lowercase().contains("unknown interaction")
                            || err_str.to_lowercase().contains("expired");
                        let fallback_ch = if *state == State::Fresh {
                            if expired {
                                *channel_id
                            } else {
                                None
                            }
                        } else {
                            *channel_id
                        };
                        let Some(ch) = fallback_ch else {
                            return Err(e);
                        };
                        tracing::warn!(error = %e, "Interaction reply failed, falling back to channel message");
                        let note_ephemeral = if ephemeral {
                            "_(this was meant to be private, but the interaction expired so I'm posting publicly)_\n"
                        } else {
                            ""
                        };
                        let mut fallback_embeds = embeds.clone();
                        if !note_ephemeral.is_empty() {
                            // Prepend downgrade notice to first embed description.
                            if let Some(first) = fallback_embeds.first() {
                                let mut f = first.clone();
                                let desc = f.description.clone().unwrap_or_default();
                                f.description = Some(format!("{note_ephemeral}{desc}"));
                                fallback_embeds[0] = f;
                            }
                        }
                        // Scrub embeds (defence-in-depth; errors may echo IDs).
                        ctx.http
                            .create_message(ch)
                            .embeds(&fallback_embeds)
                            .components(&components)
                            .allowed_mentions(Some(&mentions))
                            .await?;
                        *state = State::Responded;
                        Ok(())
                    }
                }
            }
        }
    }

    pub async fn info(&self, ctx: &Context, tone: Tone, title: &str, body: &str, ephemeral: bool) {
        if let Err(e) = self
            .embed(ctx, brand::embed(tone, title, body), ephemeral)
            .await
        {
            tracing::warn!(error = %e, title, "Failed to send reply");
        }
    }

    /// Show a friendly, actionable error with a reference ID; log the
    /// full detail under that ID and alert the operator channel if needed.
    pub async fn error(
        &self,
        ctx: &Context,
        err: &BotError,
        source: Option<KeySource>,
        request_id: &str,
    ) {
        let reference = format!("ERR-{}", utils::short_id(6));
        if err.is_operator_relevant() {
            tracing::error!(%reference, request_id, code = err.code(), error = %err, "Request failed");
        } else {
            tracing::warn!(%reference, request_id, code = err.code(), error = %err, "Request failed (user-facing)");
        }
        let body = format!(
            "{}\n\n`{}` · ref `{reference}`",
            err.hint(source, ctx.prefix()),
            err.code()
        );
        let embed = brand::embed(Tone::Error, err.title(), &body);
        if let Err(e) = self.embed(ctx, embed, true).await {
            tracing::warn!(error = %e, %reference, "Failed to deliver error message");
        }
        if err.is_operator_relevant() {
            report_to_operator(ctx, &reference, request_id, err).await;
        }
    }
}

pub async fn report_to_operator(ctx: &Context, reference: &str, request_id: &str, err: &BotError) {
    let Some(ch) = ctx.config.error_log_channel_id else {
        return;
    };
    let Some(ch_id) = Id::new_checked(ch) else {
        tracing::warn!(
            channel = ch,
            "ERROR_LOG_CHANNEL_ID is zero/invalid, skipping operator report"
        );
        return;
    };
    // Scrub before posting: downstream errors may echo request content.
    let (scrubbed, _) = utils::scrub_google_keys(&err.to_string());
    let body = format!(
        "**code** `{}`\n**request** `{request_id}`\n```\n{}\n```",
        err.code(),
        utils::truncate_chars(&scrubbed, 1500)
    );
    let embed = brand::embed(Tone::Error, &format!("Error {reference}"), &body);
    let mentions = no_mentions();
    if let Err(e) = ctx
        .http
        .create_message(ch_id)
        .embeds(&[embed])
        .allowed_mentions(Some(&mentions))
        .await
    {
        tracing::warn!(error = %e, "Failed to post to ERROR_LOG_CHANNEL_ID");
    }
}

/// Open (or reuse) a DM channel and send an embed with optional components.
pub async fn dm(
    ctx: &Context,
    user_id: Id<UserMarker>,
    embed: Embed,
    components: Vec<Component>,
) -> Result<Id<ChannelMarker>, BotError> {
    let channel = ctx
        .http
        .create_private_channel(user_id)
        .await?
        .model()
        .await?;
    let mentions = no_mentions();
    ctx.http
        .create_message(channel.id)
        .embeds(&[embed])
        .components(&components)
        .allowed_mentions(Some(&mentions))
        .await?;
    Ok(channel.id)
}

/// Post a line to a guild's configured log channel (audit of agent actions).
pub async fn guild_log(ctx: &Context, channel_id: u64, embed: Embed) {
    let Some(ch_id) = Id::new_checked(channel_id) else {
        tracing::warn!(channel_id, "guild log channel id is zero/invalid, skipping");
        return;
    };
    let mentions = no_mentions();
    if let Err(e) = ctx
        .http
        .create_message(ch_id)
        .embeds(&[embed])
        .allowed_mentions(Some(&mentions))
        .await
    {
        tracing::debug!(error = %e, channel_id, "Failed to post to guild log channel");
    }
}
