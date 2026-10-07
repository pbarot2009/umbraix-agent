use serde_json::Value;
use std::time::Duration;

/// Classified Gemini failure. Classification drives retry policy and the
/// user-facing explanation (e.g. "your key is invalid, run !byok-user").
#[derive(Debug, Clone, thiserror::Error)]
pub enum GeminiError {
    #[error("the Gemini API key was rejected as invalid")]
    InvalidKey,
    #[error("the Gemini API key is not allowed to do this: {0}")]
    PermissionDenied(String),
    #[error("Gemini rate limit hit (requests/tokens per minute)")]
    RateLimited { retry_after: Option<Duration> },
    #[error("Gemini quota exhausted: {0}")]
    QuotaExhausted(String),
    #[error("model not found or not available: {0}")]
    ModelNotFound(String),
    #[error("Gemini rejected the request: {0}")]
    BadRequest(String),
    #[error("Gemini billing/precondition problem: {0}")]
    Precondition(String),
    #[error("Gemini is temporarily unavailable (HTTP {status})")]
    Unavailable { status: u16, detail: String },
    #[error("the Gemini request timed out")]
    Timeout,
    #[error("network error talking to Gemini: {0}")]
    Network(String),
    #[error("Gemini blocked the response: {0}")]
    Blocked(String),
    #[error("Gemini returned a malformed response: {0}")]
    Malformed(String),
    #[error("this API key is paused after repeated failures ({reason}); retry in {}s", .remaining.as_secs().max(1))]
    KeyCoolingDown {
        reason: &'static str,
        remaining: Duration,
    },
}

impl GeminiError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            GeminiError::RateLimited { .. }
                | GeminiError::Unavailable { .. }
                | GeminiError::Timeout
                | GeminiError::Network(_)
        )
    }

    /// Errors that mean the key itself is unusable for a while; the key gate
    /// pauses such keys so we don't hammer Google with doomed requests.
    pub fn key_cooldown(&self) -> Option<(&'static str, Duration)> {
        match self {
            GeminiError::InvalidKey => Some(("invalid key", Duration::from_secs(600))),
            GeminiError::PermissionDenied(_) => {
                Some(("permission denied", Duration::from_secs(300)))
            }
            GeminiError::QuotaExhausted(_) => Some(("quota exhausted", Duration::from_secs(900))),
            GeminiError::Precondition(_) => Some(("billing problem", Duration::from_secs(600))),
            _ => None,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            GeminiError::InvalidKey => "GEMINI_INVALID_KEY",
            GeminiError::PermissionDenied(_) => "GEMINI_PERMISSION",
            GeminiError::RateLimited { .. } => "GEMINI_RATE_LIMIT",
            GeminiError::QuotaExhausted(_) => "GEMINI_QUOTA",
            GeminiError::ModelNotFound(_) => "GEMINI_MODEL",
            GeminiError::BadRequest(_) => "GEMINI_BAD_REQUEST",
            GeminiError::Precondition(_) => "GEMINI_BILLING",
            GeminiError::Unavailable { .. } => "GEMINI_UNAVAILABLE",
            GeminiError::Timeout => "GEMINI_TIMEOUT",
            GeminiError::Network(_) => "GEMINI_NETWORK",
            GeminiError::Blocked(_) => "GEMINI_BLOCKED",
            GeminiError::Malformed(_) => "GEMINI_MALFORMED",
            GeminiError::KeyCoolingDown { .. } => "GEMINI_KEY_PAUSED",
        }
    }

    /// Classify a non-2xx HTTP response.
    pub fn from_response(status: u16, body: &Value, retry_after_header: Option<Duration>) -> Self {
        let err = body.get("error").unwrap_or(&Value::Null);
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let rpc_status = err.get("status").and_then(|s| s.as_str()).unwrap_or("");
        let details = err
            .get("details")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        let reason = details
            .iter()
            .find_map(|d| d.get("reason").and_then(|r| r.as_str()))
            .unwrap_or("");
        let short = clip(&message, 300);

        let key_invalid = reason == "API_KEY_INVALID"
            || message.contains("API key not valid")
            || message.contains("API_KEY_INVALID")
            || message.contains("API key expired");

        match status {
            400 if key_invalid => GeminiError::InvalidKey,
            400 if rpc_status == "FAILED_PRECONDITION" => GeminiError::Precondition(short),
            400 => GeminiError::BadRequest(short),
            401 => GeminiError::InvalidKey,
            403 if key_invalid || reason == "API_KEY_SERVICE_BLOCKED" => GeminiError::InvalidKey,
            403 => GeminiError::PermissionDenied(short),
            404 => GeminiError::ModelNotFound(short),
            408 => GeminiError::Timeout,
            429 => {
                if is_daily_quota(&details, &message) {
                    GeminiError::QuotaExhausted(short)
                } else {
                    GeminiError::RateLimited {
                        retry_after: retry_delay(&details).or(retry_after_header),
                    }
                }
            }
            s if s >= 500 => GeminiError::Unavailable {
                status: s,
                detail: short,
            },
            _ => GeminiError::BadRequest(format!("HTTP {status}: {short}")),
        }
    }

    pub fn from_reqwest(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            GeminiError::Timeout
        } else {
            GeminiError::Network(e.without_url().to_string())
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    crate::utils::truncate_chars(s, max)
}

/// Daily (per-day) quota violations don't clear for hours: not retryable.
fn is_daily_quota(details: &[Value], message: &str) -> bool {
    let in_details = details.iter().any(|d| {
        d.get("violations")
            .and_then(|v| v.as_array())
            .map(|vs| {
                vs.iter().any(|v| {
                    v.get("quotaId")
                        .and_then(|q| q.as_str())
                        .map(|q| q.contains("PerDay"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });
    in_details || message.contains("per day") || message.contains("PerDay")
}

/// Parse `RetryInfo.retryDelay` (e.g. `"17s"` or `"1.5s"`).
fn retry_delay(details: &[Value]) -> Option<Duration> {
    details.iter().find_map(|d| {
        let raw = d.get("retryDelay")?.as_str()?;
        let secs: f64 = raw.trim_end_matches('s').parse().ok()?;
        (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs.min(300.0)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_invalid_key() {
        let body = json!({"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT",
            "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "API_KEY_INVALID"}]}});
        assert!(matches!(
            GeminiError::from_response(400, &body, None),
            GeminiError::InvalidKey
        ));
    }

    #[test]
    fn classifies_rate_limit_with_retry_delay() {
        let body = json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "Resource exhausted",
            "details": [{"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "17s"}]}});
        match GeminiError::from_response(429, &body, None) {
            GeminiError::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(17)))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn classifies_daily_quota_as_non_retryable() {
        let body = json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "Quota exceeded",
            "details": [{"@type": "type.googleapis.com/google.rpc.QuotaFailure",
              "violations": [{"quotaId": "GenerateRequestsPerDayPerProjectPerModel-FreeTier"}]}]}});
        let e = GeminiError::from_response(429, &body, None);
        assert!(matches!(e, GeminiError::QuotaExhausted(_)));
        assert!(!e.is_retryable());
        assert!(e.key_cooldown().is_some());
    }

    #[test]
    fn server_errors_are_retryable() {
        let e = GeminiError::from_response(503, &json!({}), None);
        assert!(e.is_retryable());
        assert!(!GeminiError::from_response(400, &json!({}), None).is_retryable());
        assert!(matches!(
            GeminiError::from_response(404, &json!({}), None),
            GeminiError::ModelNotFound(_)
        ));
    }
}
