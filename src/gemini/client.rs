use serde_json::{json, Value};
use std::time::Duration;

/// Thin wrapper around the Gemini `generateContent` REST endpoint.
///
/// Handles URL construction, `429` retries with exponential backoff
/// (up to 3 attempts), and surfacing Gemini-side error payloads.
#[derive(Clone)]
pub struct GeminiClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl GeminiClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            model,
        }
    }

    fn url(&self) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
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
            "generationConfig": { "temperature": temperature }
        });
        self.post_with_retry(&body).await
    }

    /// Follow-up call carrying tool results back (no temperature override,
    /// matching the original bot's final-pass shape).
    pub async fn generate_follow_up(
        &self,
        contents: &[Value],
        tools: &Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let body = json!({
            "contents": contents,
            "tools": tools
        });
        self.post_with_retry(&body).await
    }

    async fn post_with_retry(
        &self,
        body: &Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let url = self.url();
        let mut last_error: Option<String> = None;

        for attempt in 0..3 {
            if attempt > 0 {
                let backoff = Duration::from_millis(500 * (1 << attempt));
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
                .json(body)
                .send()
                .await?;

            let status = resp.status();
            let value: Value = resp.json().await.unwrap_or(Value::Null);

            if status.is_success() {
                return Ok(value);
            }

            // Retry only on 429 / 5xx; surface everything else immediately.
            let retryable =
                status.as_u16() == 429 || status.as_u16() == 503 || status.is_server_error();
            last_error = Some(format!("Gemini HTTP {status}: {value}"));
            if !retryable {
                break;
            }
        }

        Err(last_error
            .unwrap_or_else(|| "Gemini request failed".to_string())
            .into())
    }
}
