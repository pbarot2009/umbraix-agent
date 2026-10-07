use dashmap::DashMap;
use serde_json::Value;
use std::{
    ops::Deref,
    sync::{atomic::AtomicBool, Arc, OnceLock},
    time::{Duration, Instant},
};
use twilight_cache_inmemory::{DefaultInMemoryCache, ResourceType};
use twilight_http::Client as DiscordHttp;
use twilight_model::id::{
    marker::{ApplicationMarker, GuildMarker, UserMarker},
    Id,
};

use crate::{
    agent::memory::ConversationMemory,
    concurrency::{Metrics, TurnLimiter},
    config::Config,
    gemini::GeminiClient,
    storage::{ApiKey, Store},
    tools,
    utils::RateLimiter,
};

/// Where a pending DM-based BYOK submission will be stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByokScope {
    User,
    Server(Id<GuildMarker>),
}

#[derive(Debug, Clone, Copy)]
pub struct PendingByok {
    pub scope: ByokScope,
    pub expires: Instant,
}

/// Shared runtime state passed to every handler.
pub struct Inner {
    pub http: DiscordHttp,
    pub cache: DefaultInMemoryCache,
    pub gemini: GeminiClient,
    pub config: Config,
    pub memory: ConversationMemory,
    pub rate_limiter: RateLimiter,
    pub store: Store,
    pub limiter: TurnLimiter,
    pub metrics: Metrics,
    pub tools_decl: Value,
    pub owner_key: ApiKey,
    pub byok_sessions: DashMap<u64, PendingByok>,
    pub shard_latency: DashMap<u32, Duration>,
    pub app_id: OnceLock<Id<ApplicationMarker>>,
    pub bot_user_id: OnceLock<Id<UserMarker>>,
    pub commands_registered: AtomicBool,
    pub shutting_down: AtomicBool,
    pub started_at: Instant,
}

#[derive(Clone)]
pub struct Context {
    inner: Arc<Inner>,
}

impl Context {
    pub fn new(config: Config, store: Store) -> Self {
        let gemini = GeminiClient::new(
            Duration::from_secs(config.gemini_request_timeout_secs),
            config.gemini_max_retries,
            config.max_concurrent_per_key,
        );
        let cache = DefaultInMemoryCache::builder()
            .resource_types(
                ResourceType::GUILD
                    | ResourceType::CHANNEL
                    | ResourceType::ROLE
                    | ResourceType::MEMBER
                    | ResourceType::USER_CURRENT,
            )
            .build();
        Self {
            inner: Arc::new(Inner {
                http: DiscordHttp::new(config.discord_token.clone()),
                cache,
                gemini,
                memory: ConversationMemory::new(config.history_limit),
                rate_limiter: RateLimiter::new(config.rate_limit_secs),
                limiter: TurnLimiter::new(
                    config.max_concurrent_turns,
                    Duration::from_secs(config.queue_timeout_secs),
                ),
                store,
                metrics: Metrics::default(),
                tools_decl: tools::build_tools_declaration(),
                owner_key: ApiKey::new(config.gemini_api_key.clone()),
                byok_sessions: DashMap::new(),
                shard_latency: DashMap::new(),
                app_id: OnceLock::new(),
                bot_user_id: OnceLock::new(),
                commands_registered: AtomicBool::new(false),
                shutting_down: AtomicBool::new(false),
                started_at: Instant::now(),
                config,
            }),
        }
    }

    pub fn is_owner(&self, user_id: u64) -> bool {
        user_id == self.config.owner_id
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn set_shutting_down(&self) {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Record IDs from the first Ready event; idempotent across shards.
    pub fn init_ids(&self, app_id: Id<ApplicationMarker>, bot_user_id: Id<UserMarker>) {
        let _ = self.app_id.set(app_id);
        let _ = self.bot_user_id.set(bot_user_id);
    }

    pub fn prefix(&self) -> &str {
        &self.config.command_prefix
    }
}

impl Deref for Context {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
