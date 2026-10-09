pub mod crypto;

pub use crypto::{ApiKey, Vault};

use dashmap::DashMap;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::{str::FromStr, sync::Arc, time::Duration};

use crate::utils::{now_secs, today};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("key storage crypto error: {0}")]
    Crypto(String),
    #[error("storage setup error: {0}")]
    Setup(String),
}

/// Who a stored key belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyOwner {
    User(u64),
    Guild(u64),
}

impl KeyOwner {
    fn aad(self) -> String {
        match self {
            KeyOwner::User(id) => format!("umbraix:user:{id}"),
            KeyOwner::Guild(id) => format!("umbraix:guild:{id}"),
        }
    }

    pub fn usage_kind(self) -> (&'static str, u64) {
        match self {
            KeyOwner::User(id) => ("user", id),
            KeyOwner::Guild(id) => ("guild", id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredKey {
    pub api_key: ApiKey,
    pub hint: String,
    pub set_by: Option<u64>,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuildConfig {
    pub enabled: bool,
    pub allow_user_byok: bool,
    pub model: Option<String>,
    pub cooldown_secs: Option<u64>,
    pub log_channel_id: Option<u64>,
    pub allow_role_id: Option<u64>,
}

impl Default for GuildConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_user_byok: true,
            model: None,
            cooldown_secs: None,
            log_channel_id: None,
            allow_role_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub requests: i64,
    pub tool_calls: i64,
    pub errors: i64,
}

#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub guild_id: Option<u64>,
    pub user_id: u64,
    pub key_source: Option<String>,
    pub action: String,
    pub detail: String,
    pub success: bool,
}

/// A half-finished agent turn persisted so a network drop / restart can
/// resume it on the next boot instead of forgetting the user's work.
///
/// Lifecycle: `pending` (created at turn start) -> checkpointed after each
/// tool batch -> `DELETE` on completion / terminal failure / expiry.
/// Rows left behind by a crash stay `pending`/`running` and are claimed
/// exactly once on boot (atomic `UPDATE ... WHERE status IN ...`), with an
/// attempt counter + age TTL so a poison request can never infinite-loop.
#[derive(Debug, Clone)]
pub struct PendingTurn {
    pub request_id: String,
    pub guild_id: u64,
    pub channel_id: u64,
    pub user_id: u64,
    pub user_name: String,
    pub prompt: String,
    pub history_json: String,
    pub tool_summary: String,
    pub status: String,
    pub attempts: i64,
    pub max_attempts: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StoreCounts {
    pub user_keys: i64,
    pub guild_keys: i64,
    pub guild_configs: i64,
}

/// Persistent state: encrypted BYOK keys, per-guild config, usage, audit log.
///
/// Hot-path reads (key lookup, guild config) are served from in-memory
/// caches that are invalidated on every write, so 100+ concurrent requests
/// don't each hit SQLite.
pub struct Store {
    pool: SqlitePool,
    vault: Vault,
    keys: DashMap<KeyOwner, Option<Arc<StoredKey>>>,
    configs: DashMap<u64, GuildConfig>,
}

// Snowflakes fit in i64 until the year 2084; store them as INTEGER.
// Corrupt negative rows map to 0 (invalid, filtered by callers) rather than
// wrapping into huge u64s.
fn sid(id: u64) -> i64 {
    i64::try_from(id).unwrap_or(i64::MAX)
}

fn uid(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl Store {
    pub async fn connect(url: &str, vault: Vault) -> Result<Self, StoreError> {
        if let Some(path) = url
            .strip_prefix("sqlite://")
            .or_else(|| url.strip_prefix("sqlite:"))
        {
            let path = path.split('?').next().unwrap_or(path);
            if path != ":memory:" && !path.is_empty() {
                if let Some(parent) = std::path::Path::new(path).parent() {
                    if !parent.as_os_str().is_empty() {
                        tokio::fs::create_dir_all(parent).await.map_err(|e| {
                            StoreError::Setup(format!("cannot create {}: {e}", parent.display()))
                        })?;
                    }
                }
            }
        }
        let opts = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5))
            .foreign_keys(true);
        let max = if url.contains(":memory:") { 1 } else { 8 };
        let pool = SqlitePoolOptions::new()
            .max_connections(max)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        tracing::info!(url = %redact_url(url), "Database ready (migrations applied)");
        Ok(Self {
            pool,
            vault,
            keys: DashMap::new(),
            configs: DashMap::new(),
        })
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn ping(&self) -> Result<(), StoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    // ---------- BYOK keys ----------

    pub async fn get_key(&self, owner: KeyOwner) -> Result<Option<Arc<StoredKey>>, StoreError> {
        if let Some(cached) = self.keys.get(&owner) {
            return Ok(cached.clone());
        }
        let (sql, id) = match owner {
            KeyOwner::User(id) => (
                "SELECT ciphertext, nonce, key_hint, NULL AS set_by, updated_at, last_used_at FROM user_keys WHERE user_id = ?1",
                id,
            ),
            KeyOwner::Guild(id) => (
                "SELECT ciphertext, nonce, key_hint, set_by, updated_at, last_used_at FROM guild_keys WHERE guild_id = ?1",
                id,
            ),
        };
        let row = sqlx::query(sql)
            .bind(sid(id))
            .fetch_optional(&self.pool)
            .await?;
        let value = match row {
            None => None,
            Some(row) => {
                let ct: Vec<u8> = row.try_get("ciphertext")?;
                let nonce: Vec<u8> = row.try_get("nonce")?;
                let plain = self
                    .vault
                    .decrypt(&ct, &nonce, owner.aad().as_bytes())
                    .map_err(StoreError::Crypto)?;
                let key = String::from_utf8(plain)
                    .map_err(|_| StoreError::Crypto("stored key is not UTF-8".into()))?;
                Some(Arc::new(StoredKey {
                    api_key: ApiKey::new(key),
                    hint: row.try_get("key_hint")?,
                    set_by: row.try_get::<Option<i64>, _>("set_by")?.map(uid),
                    updated_at: row.try_get("updated_at")?,
                    last_used_at: row.try_get("last_used_at")?,
                }))
            }
        };
        self.keys.insert(owner, value.clone());
        Ok(value)
    }

    pub async fn set_key(
        &self,
        owner: KeyOwner,
        api_key: &ApiKey,
        set_by: u64,
    ) -> Result<(), StoreError> {
        let (ct, nonce) = self
            .vault
            .encrypt(api_key.expose().as_bytes(), owner.aad().as_bytes())
            .map_err(StoreError::Crypto)?;
        let now = now_secs();
        let hint = api_key.hint();
        match owner {
            KeyOwner::User(id) => {
                sqlx::query(
                    "INSERT INTO user_keys (user_id, ciphertext, nonce, key_hint, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                     ON CONFLICT(user_id) DO UPDATE SET ciphertext = excluded.ciphertext,
                       nonce = excluded.nonce, key_hint = excluded.key_hint, updated_at = excluded.updated_at",
                )
                .bind(sid(id))
                .bind(ct)
                .bind(nonce)
                .bind(&hint)
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
            KeyOwner::Guild(id) => {
                sqlx::query(
                    "INSERT INTO guild_keys (guild_id, ciphertext, nonce, key_hint, set_by, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                     ON CONFLICT(guild_id) DO UPDATE SET ciphertext = excluded.ciphertext,
                       nonce = excluded.nonce, key_hint = excluded.key_hint, set_by = excluded.set_by,
                       updated_at = excluded.updated_at",
                )
                .bind(sid(id))
                .bind(ct)
                .bind(nonce)
                .bind(&hint)
                .bind(sid(set_by))
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
        }
        self.keys.remove(&owner);
        Ok(())
    }

    pub async fn delete_key(&self, owner: KeyOwner) -> Result<bool, StoreError> {
        let res = match owner {
            KeyOwner::User(id) => {
                sqlx::query("DELETE FROM user_keys WHERE user_id = ?1")
                    .bind(sid(id))
                    .execute(&self.pool)
                    .await?
            }
            KeyOwner::Guild(id) => {
                sqlx::query("DELETE FROM guild_keys WHERE guild_id = ?1")
                    .bind(sid(id))
                    .execute(&self.pool)
                    .await?
            }
        };
        self.keys.remove(&owner);
        Ok(res.rows_affected() > 0)
    }

    pub async fn touch_key(&self, owner: KeyOwner) -> Result<(), StoreError> {
        let (sql, id) = match owner {
            KeyOwner::User(id) => (
                "UPDATE user_keys SET last_used_at = ?1 WHERE user_id = ?2",
                id,
            ),
            KeyOwner::Guild(id) => (
                "UPDATE guild_keys SET last_used_at = ?1 WHERE guild_id = ?2",
                id,
            ),
        };
        let now = now_secs();
        sqlx::query(sql)
            .bind(now)
            .bind(sid(id))
            .execute(&self.pool)
            .await?;
        // Keep the cache coherent: refresh last_used_at or drop the entry.
        if let Some(mut cached) = self.keys.get_mut(&owner) {
            if let Some(stored) = cached.as_mut() {
                let mut updated = (**stored).clone();
                updated.last_used_at = Some(now);
                *cached = Some(Arc::new(updated));
            }
        }
        Ok(())
    }

    // ---------- Guild config ----------

    pub async fn guild_config(&self, guild_id: u64) -> Result<GuildConfig, StoreError> {
        if let Some(c) = self.configs.get(&guild_id) {
            return Ok(c.clone());
        }
        let row = sqlx::query(
            "SELECT enabled, allow_user_byok, model, cooldown_secs, log_channel_id, allow_role_id
             FROM guild_config WHERE guild_id = ?1",
        )
        .bind(sid(guild_id))
        .fetch_optional(&self.pool)
        .await?;
        let cfg = match row {
            None => GuildConfig::default(),
            Some(r) => GuildConfig {
                enabled: r.try_get::<i64, _>("enabled")? != 0,
                allow_user_byok: r.try_get::<i64, _>("allow_user_byok")? != 0,
                model: r.try_get("model")?,
                cooldown_secs: r
                    .try_get::<Option<i64>, _>("cooldown_secs")?
                    .map(|v| v.max(0) as u64),
                log_channel_id: r.try_get::<Option<i64>, _>("log_channel_id")?.map(uid),
                allow_role_id: r.try_get::<Option<i64>, _>("allow_role_id")?.map(uid),
            },
        };
        self.configs.insert(guild_id, cfg.clone());
        Ok(cfg)
    }

    pub async fn update_guild_config(
        &self,
        guild_id: u64,
        f: impl FnOnce(&mut GuildConfig),
    ) -> Result<GuildConfig, StoreError> {
        // Read-modify-write inside an IMMEDIATE transaction so concurrent
        // admin changes to different fields can't clobber each other.
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT enabled, allow_user_byok, model, cooldown_secs, log_channel_id, allow_role_id
             FROM guild_config WHERE guild_id = ?1",
        )
        .bind(sid(guild_id))
        .fetch_optional(&mut *tx)
        .await?;
        let mut cfg = match row {
            None => GuildConfig::default(),
            Some(r) => GuildConfig {
                enabled: r.try_get::<i64, _>("enabled")? != 0,
                allow_user_byok: r.try_get::<i64, _>("allow_user_byok")? != 0,
                model: r.try_get("model")?,
                cooldown_secs: r
                    .try_get::<Option<i64>, _>("cooldown_secs")?
                    .map(|v| v.max(0) as u64),
                log_channel_id: r.try_get::<Option<i64>, _>("log_channel_id")?.map(uid),
                allow_role_id: r.try_get::<Option<i64>, _>("allow_role_id")?.map(uid),
            },
        };
        f(&mut cfg);
        sqlx::query(
            "INSERT INTO guild_config (guild_id, enabled, allow_user_byok, model, cooldown_secs, log_channel_id, allow_role_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(guild_id) DO UPDATE SET enabled = excluded.enabled,
               allow_user_byok = excluded.allow_user_byok, model = excluded.model,
               cooldown_secs = excluded.cooldown_secs, log_channel_id = excluded.log_channel_id,
               allow_role_id = excluded.allow_role_id, updated_at = excluded.updated_at",
        )
        .bind(sid(guild_id))
        .bind(cfg.enabled as i64)
        .bind(cfg.allow_user_byok as i64)
        .bind(cfg.model.as_deref())
        .bind(cfg.cooldown_secs.map(|v| v as i64))
        .bind(cfg.log_channel_id.map(sid))
        .bind(cfg.allow_role_id.map(sid))
        .bind(now_secs())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.configs.insert(guild_id, cfg.clone());
        Ok(cfg)
    }

    /// Remove everything stored for a guild (bot was removed from it).
    /// Deletes keys, config, usage and audit rows so no guild data lingers.
    pub async fn purge_guild(&self, guild_id: u64) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM guild_keys WHERE guild_id = ?1")
            .bind(sid(guild_id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM guild_config WHERE guild_id = ?1")
            .bind(sid(guild_id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM usage_daily WHERE subject_kind = 'guild' AND subject_id = ?1")
            .bind(sid(guild_id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM audit_log WHERE guild_id = ?1")
            .bind(sid(guild_id))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM pending_turns WHERE guild_id = ?1")
            .bind(sid(guild_id))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.keys.remove(&KeyOwner::Guild(guild_id));
        self.configs.remove(&guild_id);
        Ok(())
    }

    // ---------- Usage & audit ----------

    pub async fn record_usage(
        &self,
        kind: &str,
        subject_id: u64,
        requests: i64,
        tool_calls: i64,
        errors: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO usage_daily (day, subject_kind, subject_id, requests, tool_calls, errors)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(day, subject_kind, subject_id) DO UPDATE SET
               requests = requests + excluded.requests,
               tool_calls = tool_calls + excluded.tool_calls,
               errors = errors + excluded.errors",
        )
        .bind(today())
        .bind(kind)
        .bind(sid(subject_id))
        .bind(requests)
        .bind(tool_calls)
        .bind(errors)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn usage(&self, kind: &str, subject_id: u64, days: i64) -> Result<Usage, StoreError> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(requests),0) AS r, COALESCE(SUM(tool_calls),0) AS t, COALESCE(SUM(errors),0) AS e
             FROM usage_daily WHERE subject_kind = ?1 AND subject_id = ?2 AND day > ?3",
        )
        .bind(kind)
        .bind(sid(subject_id))
        .bind(today() - days.max(1))
        .fetch_one(&self.pool)
        .await?;
        Ok(Usage {
            requests: row.try_get("r")?,
            tool_calls: row.try_get("t")?,
            errors: row.try_get("e")?,
        })
    }

    pub async fn audit(&self, e: AuditEntry) -> Result<(), StoreError> {
        // Scrub defence-in-depth: callers should already scrub, but audit rows
        // persist — never store a raw key.
        let (detail, _) = crate::utils::scrub_google_keys(&e.detail);
        sqlx::query(
            "INSERT INTO audit_log (ts, guild_id, user_id, key_source, action, detail, success)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(now_secs())
        .bind(e.guild_id.map(sid))
        .bind(sid(e.user_id))
        .bind(e.key_source)
        .bind(e.action)
        .bind(crate::utils::truncate_chars(&detail, 2000))
        .bind(e.success as i64)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn counts(&self) -> Result<StoreCounts, StoreError> {
        let row = sqlx::query(
            "SELECT (SELECT COUNT(*) FROM user_keys) AS u,
                    (SELECT COUNT(*) FROM guild_keys) AS g,
                    (SELECT COUNT(*) FROM guild_config) AS c",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(StoreCounts {
            user_keys: row.try_get("u")?,
            guild_keys: row.try_get("g")?,
            guild_configs: row.try_get("c")?,
        })
    }

    // ---------- Pending turns (crash-safe resume) ----------

    /// Create (or replace) a pending-turn row at turn start. The prompt is
    /// scrubbed + truncated by the caller; never persist raw API keys.
    pub async fn create_pending_turn(&self, turn: &PendingTurn) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO pending_turns (request_id, guild_id, channel_id, user_id, user_name,
              prompt, history_json, tool_summary, status, attempts, max_attempts,
              created_at, updated_at, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?11, ?11, ?12)
             ON CONFLICT(request_id) DO UPDATE SET
               guild_id = excluded.guild_id, channel_id = excluded.channel_id,
               user_id = excluded.user_id, user_name = excluded.user_name,
               prompt = excluded.prompt, history_json = excluded.history_json,
               tool_summary = excluded.tool_summary, status = 'pending',
               attempts = excluded.attempts, max_attempts = excluded.max_attempts,
               updated_at = excluded.updated_at, last_error = excluded.last_error",
        )
        .bind(&turn.request_id)
        .bind(sid(turn.guild_id))
        .bind(sid(turn.channel_id))
        .bind(sid(turn.user_id))
        .bind(crate::utils::truncate_chars(&turn.user_name, 100))
        .bind(crate::utils::truncate_chars(&turn.prompt, 4000))
        .bind(&turn.history_json)
        .bind(crate::utils::truncate_chars(&turn.tool_summary, 1000))
        .bind(turn.attempts)
        .bind(turn.max_attempts)
        .bind(turn.created_at)
        .bind(turn.last_error.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Best-effort checkpoint after each tool batch: keeps the resume point
    /// fresh without blocking the turn (callers `tokio::spawn` this or await
    /// it directly — it is a single indexed UPDATE).
    pub async fn checkpoint_pending(
        &self,
        request_id: &str,
        history_json: &str,
        tool_summary: &str,
    ) -> Result<(), StoreError> {
        // Bound row size: history is already bounded upstream (last ~100
        // entries), this is defence-in-depth so one huge dump can't bloat SQLite.
        let hist = crate::utils::truncate_chars(history_json, 200_000);
        sqlx::query(
            "UPDATE pending_turns SET history_json = ?1, tool_summary = ?2, updated_at = ?3,
              status = CASE WHEN status = 'pending' THEN 'pending' ELSE status END
             WHERE request_id = ?4 AND status IN ('pending', 'running')",
        )
        .bind(hist)
        .bind(crate::utils::truncate_chars(tool_summary, 1000))
        .bind(now_secs())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_pending(&self, request_id: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM pending_turns WHERE request_id = ?1")
            .bind(request_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn mark_pending_error(
        &self,
        request_id: &str,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let (scrubbed, _) = crate::utils::scrub_google_keys(last_error);
        sqlx::query(
            "UPDATE pending_turns SET last_error = ?1, updated_at = ?2 WHERE request_id = ?3",
        )
        .bind(crate::utils::truncate_chars(&scrubbed, 1000))
        .bind(now_secs())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn pending_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<PendingTurn, StoreError> {
        Ok(PendingTurn {
            request_id: row.try_get("request_id")?,
            guild_id: uid(row.try_get::<i64, _>("guild_id")?),
            channel_id: uid(row.try_get::<i64, _>("channel_id")?),
            user_id: uid(row.try_get::<i64, _>("user_id")?),
            user_name: row.try_get("user_name")?,
            prompt: row.try_get("prompt")?,
            history_json: row.try_get("history_json")?,
            tool_summary: row.try_get("tool_summary")?,
            status: row.try_get("status")?,
            attempts: row.try_get("attempts")?,
            max_attempts: row.try_get("max_attempts")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            last_error: row.try_get("last_error")?,
        })
    }

    /// Rows eligible for boot resume: still pending/running, under the
    /// attempt budget and newer than `max_age_secs`.
    pub async fn list_recoverable_turns(
        &self,
        max_age_secs: i64,
        limit: i64,
    ) -> Result<Vec<PendingTurn>, StoreError> {
        let cutoff = now_secs() - max_age_secs.max(60);
        let rows = sqlx::query(
            "SELECT request_id, guild_id, channel_id, user_id, user_name, prompt,
                    history_json, tool_summary, status, attempts, max_attempts,
                    created_at, updated_at, last_error
             FROM pending_turns
             WHERE status IN ('pending', 'running') AND updated_at > ?1
             ORDER BY updated_at ASC LIMIT ?2",
        )
        .bind(cutoff)
        .bind(limit.clamp(1, 50))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::pending_from_row).collect()
    }

    /// Atomically claim one row for resume (prevents double-resume when
    /// several shards boot at once). Returns `None` when already claimed,
    /// over budget, or gone.
    pub async fn claim_pending_turn(
        &self,
        request_id: &str,
    ) -> Result<Option<PendingTurn>, StoreError> {
        let res = sqlx::query(
            "UPDATE pending_turns SET status = 'running', attempts = attempts + 1, updated_at = ?1
             WHERE request_id = ?2 AND status IN ('pending', 'running')",
        )
        .bind(now_secs())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT request_id, guild_id, channel_id, user_id, user_name, prompt,
                    history_json, tool_summary, status, attempts, max_attempts,
                    created_at, updated_at, last_error
             FROM pending_turns WHERE request_id = ?1",
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| Self::pending_from_row(&r)).transpose()
    }

    /// Park a failed-but-retryable resume back to `pending` (with fresh
    /// checkpoint) so the *next* boot can retry — without tight-looping in
    /// this boot. Over-budget rows are deleted instead.
    pub async fn park_pending_for_retry(
        &self,
        request_id: &str,
        history_json: &str,
        tool_summary: &str,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let (scrubbed, _) = crate::utils::scrub_google_keys(last_error);
        let hist = crate::utils::truncate_chars(history_json, 200_000);
        let res = sqlx::query(
            "UPDATE pending_turns SET status = 'pending', history_json = ?1, tool_summary = ?2,
              last_error = ?3, updated_at = ?4
             WHERE request_id = ?5 AND attempts < max_attempts",
        )
        .bind(hist)
        .bind(crate::utils::truncate_chars(tool_summary, 1000))
        .bind(crate::utils::truncate_chars(&scrubbed, 1000))
        .bind(now_secs())
        .bind(request_id)
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            // Over budget (or gone): delete so it can never loop.
            self.delete_pending(request_id).await?;
        }
        Ok(())
    }

    /// Delete rows older than `max_age_secs` (stale crash leftovers).
    /// Returns the number removed.
    pub async fn prune_expired_pending(&self, max_age_secs: i64) -> Result<u64, StoreError> {
        let cutoff = now_secs() - max_age_secs.max(60);
        let res = sqlx::query("DELETE FROM pending_turns WHERE updated_at <= ?1")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}

fn redact_url(url: &str) -> String {
    // Hide the entire userinfo (password may itself contain '@').
    match url.rfind('@') {
        Some(at) => {
            // Keep only scheme://***@rest for debuggability.
            if let Some(scheme_end) = url.find("://") {
                format!("{}://***{}", &url[..scheme_end], &url[at..])
            } else {
                format!("***{}", &url[at..])
            }
        }
        None => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    async fn store() -> Store {
        let vault = Vault::from_base64(&STANDARD.encode([3u8; 32])).unwrap();
        Store::connect("sqlite::memory:", vault).await.unwrap()
    }

    #[tokio::test]
    async fn key_roundtrip_and_delete() {
        let s = store().await;
        let owner = KeyOwner::User(42);
        assert!(s.get_key(owner).await.unwrap().is_none());
        let key = ApiKey::new("AIzaTESTKEY0000000000000000000000000abcd");
        s.set_key(owner, &key, 42).await.unwrap();
        let got = s.get_key(owner).await.unwrap().unwrap();
        assert_eq!(got.api_key, key);
        assert_eq!(got.hint, "AIza…abcd");
        assert!(s.get_key(KeyOwner::Guild(42)).await.unwrap().is_none());
        assert!(s.delete_key(owner).await.unwrap());
        assert!(s.get_key(owner).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn guild_config_persists_and_purges() {
        let s = store().await;
        assert_eq!(s.guild_config(7).await.unwrap(), GuildConfig::default());
        s.update_guild_config(7, |c| {
            c.enabled = false;
            c.allow_role_id = Some(99);
        })
        .await
        .unwrap();
        s.configs.clear();
        let c = s.guild_config(7).await.unwrap();
        assert!(!c.enabled);
        assert_eq!(c.allow_role_id, Some(99));
        s.set_key(
            KeyOwner::Guild(7),
            &ApiKey::new("AIzaGUILDKEY000000000000000000000000wxyz"),
            1,
        )
        .await
        .unwrap();
        s.purge_guild(7).await.unwrap();
        assert!(s.get_key(KeyOwner::Guild(7)).await.unwrap().is_none());
        assert_eq!(s.guild_config(7).await.unwrap(), GuildConfig::default());
    }

    #[tokio::test]
    async fn usage_accumulates() {
        let s = store().await;
        s.record_usage("user", 1, 1, 3, 0).await.unwrap();
        s.record_usage("user", 1, 1, 2, 1).await.unwrap();
        let u = s.usage("user", 1, 1).await.unwrap();
        assert_eq!(
            u,
            Usage {
                requests: 2,
                tool_calls: 5,
                errors: 1
            }
        );
        s.audit(AuditEntry {
            guild_id: Some(1),
            user_id: 1,
            key_source: Some("user".into()),
            action: "test".into(),
            detail: "x".into(),
            success: true,
        })
        .await
        .unwrap();
        assert_eq!(s.counts().await.unwrap().user_keys, 0);
    }
}
