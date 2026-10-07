use serde_json::{json, Value};
use std::time::Duration;

/// Thin wrapper around the Gemini `generateContent` REST endpoint.
///
/// Handles URL construction, `429` retries with exponential backoff
/// (up to 4 attempts, honoring `Retry-After` when present), request
/// timeouts, and surfacing Gemini-side error payloads.
///
/// Security note: the API key is sent via the `x-goog-api-key` header
/// instead of a `?key=` query parameter so it never appears in URLs,
/// access logs, or error strings.
#[derive(Clone)]
pub struct GeminiClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl GeminiClient {
    pub fn new(api_key: String, model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            api_key,
            model,
        }
    }

    fn url(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        )
    }

    /// Single `generateContent` call with tools + temperature.
    pub async fn generate(
        &self,
        contents: &[Value],
        tools: &Value,
        temperature: f32,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "contents": contents,
            "tools": tools,
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": 2048,
            }
        });
        self.post_with_retry(&body).await
    }

    /// Follow-up call carrying tool results back.
    ///
    /// Temperature is forwarded (unlike the old implementation which dropped
    /// it, causing non-deterministic follow-ups at the model default).
    pub async fn generate_follow_up(
        &self,
        contents: &[Value],
        tools: &Value,
        temperature: f32,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "contents": contents,
            "tools": tools,
            "generationConfig": {
                "temperature": temperature,
                "maxOutputTokens": 2048,
            }
        });
        self.post_with_retry(&body).await
    }

    async fn post_with_retry(
        &self,
        body: &Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let url = self.url();
        let mut last_error: Option<String> = None;

        for attempt in 0..4 {
            if attempt > 0 {
                // Exponential backoff with jitter: 1s, 2s, 4s (+0-500ms).
                let base = 1000u64.saturating_mul(1 << (attempt - 1)).min(8000);
                let jitter = (attempt as u64 * 137) % 500;
                let backoff = Duration::from_millis(base + jitter);
                tracing::warn!(
                    attempt,
                    ?backoff,
                    "Gemini rate-limited, retrying with backoff"
                );
                tokio::time::sleep(backoff).await;
            }

            let resp = self
                .http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("x-goog-api-key", &self.api_key)
                .json(body)
                .send()
                .await?;

            let status = resp.status();
            // Honor Retry-After on 429 when present (checked before consuming body).
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs);

            let raw = resp.bytes().await.unwrap_or_default();
            let value: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);

            if status.is_success() {
                // Detect blocked/empty responses early so the caller can
                // surface a useful message instead of "no candidates".
                if let Some(reason) = block_reason(&value) {
                    return Err(format!("Gemini blocked the response: {reason}").into());
                }
                return Ok(value);
            }

            // Truncate huge error payloads; never include the API key.
            let mut detail = value.to_string();
            if detail.len() > 2000 {
                detail.truncate(2000);
                detail.push_str("…(truncated)");
            }
            // Retry only on 429 / 5xx; surface everything else immediately.
            let retryable =
                status.as_u16() == 429 || status.as_u16() == 503 || status.is_server_error();
            last_error = Some(format!("Gemini HTTP {status}: {detail}"));
            if !retryable {
                break;
            }
            if let Some(delay) = retry_after {
                tracing::warn!(?delay, "Gemini asked to retry-after delay");
                tokio::time::sleep(delay.min(Duration::from_secs(60))).await;
            }
        }

        Err(last_error
            .unwrap_or_else(|| "Gemini request failed".to_string())
            .into())
    }
}

/// Surface `promptFeedback.blockReason` / empty-candidate blocks, if any.
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
                if reason.eq_ignore_ascii_case("SAFETY") {
                    return Some("stopped for safety".to_string());
                }
            }
        }
    }
    None
}
