use dashmap::DashMap;
use rand::Rng;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

use super::error::GeminiError;
use crate::storage::ApiKey;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const BACKOFF_BASE_MS: u64 = 1_000;
const BACKOFF_CAP_MS: u64 = 30_000;
const MAX_SERVER_DELAY: Duration = Duration::from_secs(60);

/// Shared Gemini REST client used for every key (owner, user, server).
///
/// - One pooled `reqwest::Client` for all requests (keep-alive, HTTP/2).
/// - Per-key concurrency gate so one busy key can't starve others and we
///   stay under each project's RPM.
/// - Per-key circuit breaker: invalid/quota-exhausted keys are paused
///   instead of retried.
/// - Retries only transient errors (429 per-minute, 408, 5xx, network) with
///   exponential backoff + full jitter, honoring `RetryInfo`/`Retry-After`.
/// - The key travels in the `x-goog-api-key` header, never in URLs/logs.
pub struct GeminiClient {
    http: reqwest::Client,
    max_retries: u32,
    per_key_limit: usize,
    gates: DashMap<u64, Arc<Semaphore>>,
    paused: DashMap<u64, (Instant, &'static str)>,
    pub stats: GeminiStats,
}

#[derive(Debug, Default)]
pub struct GeminiStats {
    pub requests: AtomicU64,
    pub retries: AtomicU64,
    pub failures: AtomicU64,
}

pub struct GenerateParams<'a> {
    pub api_key: &'a ApiKey,
    pub model: &'a str,
    pub system: &'a Value,
    pub contents: &'a [Value],
    pub tools: &'a Value,
    pub temperature: f32,
}

impl GeminiClient {
    pub fn new(request_timeout: Duration, max_retries: u32, per_key_limit: usize) -> Self {
        let http = reqwest::Client::builder()
            .timeout(request_timeout)
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(64)
            .user_agent(concat!("UmbraixAgent/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            max_retries,
            per_key_limit: per_key_limit.max(1),
            gates: DashMap::new(),
            paused: DashMap::new(),
            stats: GeminiStats::default(),
        }
    }

    fn gate(&self, key: &ApiKey) -> Arc<Semaphore> {
        self.gates
            .entry(key.fingerprint())
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_key_limit)))
            .clone()
    }

    fn check_paused(&self, key: &ApiKey) -> Result<(), GeminiError> {
        let fp = key.fingerprint();
        if let Some(entry) = self.paused.get(&fp) {
            let (until, reason) = *entry;
            let now = Instant::now();
            if until > now {
                return Err(GeminiError::KeyCoolingDown {
                    reason,
                    remaining: until - now,
                });
            }
            drop(entry);
            self.paused.remove(&fp);
        }
        Ok(())
    }

    /// Clear any pause for a key (called after a user saves a fresh key).
    pub fn unpause(&self, key: &ApiKey) {
        self.paused.remove(&key.fingerprint());
    }

    pub async fn generate(&self, p: GenerateParams<'_>) -> Result<Value, GeminiError> {
        self.check_paused(p.api_key)?;
        let body = json!({
            "systemInstruction": p.system,
            "contents": p.contents,
            "tools": p.tools,
            "generationConfig": {
                "temperature": p.temperature,
                "maxOutputTokens": 2048,
            }
        });
        let url = format!("{BASE_URL}/models/{}:generateContent", p.model);

        let gate = self.gate(p.api_key);
        let _permit = gate
            .acquire_owned()
            .await
            .map_err(|_| GeminiError::Network("key gate closed".into()))?;

        let result = self.post_with_retry(&url, p.api_key, &body).await;
        if let Err(e) = &result {
            self.stats.failures.fetch_add(1, Ordering::Relaxed);
            if let Some((reason, dur)) = e.key_cooldown() {
                tracing::warn!(
                    key = %p.api_key.hint(),
                    reason,
                    pause_secs = dur.as_secs(),
                    "Pausing Gemini key after non-recoverable error"
                );
                self.paused
                    .insert(p.api_key.fingerprint(), (Instant::now() + dur, reason));
            }
        }
        let value = result?;
        if let Some(reason) = block_reason(&value) {
            return Err(GeminiError::Blocked(reason));
        }
        Ok(value)
    }

    /// Cheap authenticated call used to verify a BYOK key before storing it.
    pub async fn validate_key(&self, key: &ApiKey) -> Result<(), GeminiError> {
        let resp = self
            .http
            .get(format!("{BASE_URL}/models?pageSize=1"))
            .header("x-goog-api-key", key.expose())
            .send()
            .await
            .map_err(GeminiError::from_reqwest)?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(());
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        Err(GeminiError::from_response(status, &body, None))
    }

    async fn post_with_retry(
        &self,
        url: &str,
        key: &ApiKey,
        body: &Value,
    ) -> Result<Value, GeminiError> {
        let mut attempt: u32 = 0;
        loop {
            self.stats.requests.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            let err = match self.post_once(url, key, body).await {
                Ok(v) => {
                    tracing::debug!(
                        attempt,
                        ms = started.elapsed().as_millis() as u64,
                        "Gemini call ok"
                    );
                    return Ok(v);
                }
                Err(e) => e,
            };
            if !err.is_retryable() || attempt >= self.max_retries {
                tracing::warn!(
                    attempt,
                    code = err.code(),
                    error = %err,
                    "Gemini call failed (giving up)"
                );
                return Err(err);
            }
            let hint = match &err {
                GeminiError::RateLimited { retry_after } => *retry_after,
                _ => None,
            };
            let delay = backoff_delay(attempt, hint);
            self.stats.retries.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                attempt = attempt + 1,
                max = self.max_retries,
                code = err.code(),
                delay_ms = delay.as_millis() as u64,
                "Gemini transient error, retrying"
            );
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn post_once(&self, url: &str, key: &ApiKey, body: &Value) -> Result<Value, GeminiError> {
        let resp = self
            .http
            .post(url)
            .header("x-goog-api-key", key.expose())
            .json(body)
            .send()
            .await
            .map_err(GeminiError::from_reqwest)?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let raw = resp.bytes().await.map_err(GeminiError::from_reqwest)?;
        let value: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            if value.is_null() {
                return Err(GeminiError::Malformed("empty or non-JSON body".into()));
            }
            return Ok(value);
        }
        Err(GeminiError::from_response(status, &value, retry_after))
    }
}

/// Exponential backoff with full jitter; a server-provided delay is a floor.
pub fn backoff_delay(attempt: u32, server_hint: Option<Duration>) -> Duration {
    let exp = BACKOFF_BASE_MS
        .saturating_mul(1u64 << attempt.min(10))
        .min(BACKOFF_CAP_MS);
    let jittered = rand::rng().random_range(exp / 2..=exp);
    let mut delay = Duration::from_millis(jittered);
    if let Some(hint) = server_hint {
        delay = delay.max(hint.min(MAX_SERVER_DELAY));
    }
    delay
}

/// Surface `promptFeedback.blockReason` / safety stops, if any.
fn block_reason(res: &Value) -> Option<String> {
    if let Some(reason) = res
        .get("promptFeedback")
        .and_then(|f| f.get("blockReason"))
        .and_then(|r| r.as_str())
    {
        if !reason.eq_ignore_ascii_case("BLOCK_REASON_UNSPECIFIED") {
            return Some(format!("prompt blocked ({reason})"));
        }
    }
    if let Some(candidates) = res.get("candidates").and_then(|c| c.as_array()) {
        if candidates.is_empty() {
            return Some("no candidates returned".to_string());
        }
        for cand in candidates {
            if let Some(reason) = cand.get("finishReason").and_then(|r| r.as_str()) {
                if matches!(
                    reason,
                    "SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" | "RECITATION"
                ) {
                    return Some(format!("stopped ({reason})"));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_is_capped() {
        for attempt in 0..12 {
            let d = backoff_delay(attempt, None);
            assert!(d <= Duration::from_millis(BACKOFF_CAP_MS));
            assert!(d >= Duration::from_millis(BACKOFF_BASE_MS / 2));
        }
    }

    #[test]
    fn backoff_honors_server_hint() {
        let d = backoff_delay(0, Some(Duration::from_secs(20)));
        assert!(d >= Duration::from_secs(20));
        let d = backoff_delay(0, Some(Duration::from_secs(600)));
        assert!(d <= MAX_SERVER_DELAY);
    }

    #[test]
    fn detects_safety_block() {
        let v = json!({"candidates": [{"finishReason": "SAFETY"}]});
        assert!(block_reason(&v).is_some());
        let ok = json!({"candidates": [{"finishReason": "STOP", "content": {}}]});
        assert!(block_reason(&ok).is_none());
    }

    #[test]
    fn paused_key_fails_fast() {
        let c = GeminiClient::new(Duration::from_secs(5), 0, 2);
        let key = ApiKey::new("AIzaXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX");
        c.paused.insert(
            key.fingerprint(),
            (Instant::now() + Duration::from_secs(60), "invalid key"),
        );
        assert!(matches!(
            c.check_paused(&key),
            Err(GeminiError::KeyCoolingDown { .. })
        ));
        c.unpause(&key);
        assert!(c.check_paused(&key).is_ok());
    }
}
