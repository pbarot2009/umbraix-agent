use std::{ops::Deref, sync::Arc};
use twilight_http::Client as DiscordHttp;

use crate::{
    agent::memory::ConversationMemory, config::Config, gemini::client::GeminiClient,
    utils::RateLimiter,
};

/// Shared `twilight-http` client, Gemini client, config, memory and rate limiter.
pub struct Inner {
    pub http: DiscordHttp,
    pub gemini: GeminiClient,
    pub config: Config,
    pub memory: ConversationMemory,
    pub rate_limiter: RateLimiter,
}

/// Shared, cheaply-cloneable runtime state.
///
/// Follows the twilight template convention: a thin `Arc` wrapper passed to
/// every event handler and tool executor instead of loose `Arc`s.
#[derive(Clone)]
pub struct Context {
    inner: Arc<Inner>,
}

impl Context {
    pub fn new(config: Config) -> Self {
        let gemini = GeminiClient::new(config.gemini_api_key.clone(), config.gemini_model.clone());
        Self {
            inner: Arc::new(Inner {
                http: DiscordHttp::new(config.discord_token.clone()),
                gemini,
                memory: ConversationMemory::new(config.history_limit),
                rate_limiter: RateLimiter::new(config.rate_limit_secs),
                config,
            }),
        }
    }
}

impl Deref for Context {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
