use serde_json::Value;
use std::time::Duration;

/// Classified Gemini failure. Classification drives retry policy and the
/// user-facing explanation (e.g. "your key is invalid, run !byok-user").
///
/// Every HTTP status the Generative Language API can return is mapped here —
/// previously several codes fell through to a generic message with no hint.
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
    #[error("Gemini API is disabled for this project: {0}")]
    ServiceDisabled(String),
    #[error("Gemini is not available in this region/model: {0}")]
    RegionUnsupported(String),
    #[error("request too large (context/payload limit): {0}")]
    PayloadTooLarge(String),
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
            GeminiError::ServiceDisabled(_) => {
                Some(("api disabled", Duration::from_secs(600)))
            }
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
            GeminiError::ServiceDisabled(_) => "GEMINI_SERVICE_DISABLED",
            GeminiError::RegionUnsupported(_) => "GEMINI_REGION",
            GeminiError::PayloadTooLarge(_) => "GEMINI_TOO_LARGE",
            GeminiError::Unavailable { .. } => "GEMINI_UNAVAILABLE",
            GeminiError::Timeout => "GEMINI_TIMEOUT",
            GeminiError::Network(_) => "GEMINI_NETWORK",
            GeminiError::Blocked(_) => "GEMINI_BLOCKED",
            GeminiError::Malformed(_) => "GEMINI_MALFORMED",
            GeminiError::KeyCoolingDown { .. } => "GEMINI_KEY_PAUSED",
        }
    }

    /// Classify a non-2xx HTTP response. Covers every status the Generative
    /// Language API emits (400/401/403/404/408/409/413/429/5xx) plus
    /// rpc `status` / `reason` strings, so no code falls through silently.
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
        let detail_types: Vec<&str> = details
            .iter()
            .filter_map(|d| d.get("@type").and_then(|t| t.as_str()))
            .collect();
        let reason = details
            .iter()
            .find_map(|d| d.get("reason").and_then(|r| r.as_str()))
            .unwrap_or("");
        let short = clip(&message, 300);
        let msg_lc = message.to_lowercase();

        let key_invalid = reason == "API_KEY_INVALID"
            || message.contains("API key not valid")
            || message.contains("API_KEY_INVALID")
            || message.contains("API key expired")
            || message.contains("API_KEY_SERVICE_BLOCKED")
            || reason == "API_KEY_SERVICE_BLOCKED"
            || msg_lc.contains("leaked")
            || msg_lc.contains("revoked");
        let service_disabled = reason == "SERVICE_DISABLED"
            || message.contains("SERVICE_DISABLED")
            || message.contains("has not been used in project")
            || message.contains("is disabled")
                && message.contains("Enable it")
            || message.contains("generativelanguage.googleapis.com")
                && msg_lc.contains("not enabled");
        let billing = msg_lc.contains("billing")
            || message.contains("BILLING_NOT_ENABLED")
            || reason == "BILLING_DISABLED"
            || reason == "BILLING_NOT_ENABLED";
        let region = msg_lc.contains("location")
            || msg_lc.contains("region")
            || msg_lc.contains("not available in your country")
            || msg_lc.contains("unsupported country")
            || reason == "LOCATION_POLICY_VIOLATED";
        let safety_400 = msg_lc.contains("safety")
            || msg_lc.contains("blocked")
                && (msg_lc.contains("harm") || msg_lc.contains("policy"))
            || rpc_status == "INVALID_ARGUMENT" && msg_lc.contains("prohibited");
        let too_large = msg_lc.contains("too large")
            || msg_lc.contains("max output tokens")
            || msg_lc.contains("context length")
            || msg_lc.contains("token limit")
            || msg_lc.contains("exceeds the maximum")
            || detail_types.iter().any(|t| t.contains("BadRequest"))
                && (msg_lc.contains("tokens") || msg_lc.contains("size"));

        match status {
            400 if key_invalid => GeminiError::InvalidKey,
            400 if safety_400 => GeminiError::Blocked(short),
            400 if service_disabled => GeminiError::ServiceDisabled(short),
            400 if billing => GeminiError::Precondition(short),
            400 if region => GeminiError::RegionUnsupported(short),
            400 if too_large => GeminiError::PayloadTooLarge(short),
            400 if rpc_status == "FAILED_PRECONDITION" => GeminiError::Precondition(short),
            400 => GeminiError::BadRequest(if short.is_empty() {
                "Gemini rejected the request as invalid (HTTP 400). Try `!clear` and rephrase with shorter text.".into()
            } else {
                short
            }),
            401 => GeminiError::InvalidKey,
            402 => GeminiError::Precondition(if short.is_empty() {
                "Payment required for this model/quota.".into()
            } else {
                short
            }),
            403 if key_invalid => GeminiError::InvalidKey,
            403 if service_disabled => GeminiError::ServiceDisabled(short),
            403 if billing => GeminiError::Precondition(short),
            403 if region => GeminiError::RegionUnsupported(short),
            403 => GeminiError::PermissionDenied(if short.is_empty() {
                "Access denied (HTTP 403).".into()
            } else {
                short
            }),
            404 => GeminiError::ModelNotFound(if short.is_empty() {
                "That model name doesn't exist for this key.".into()
            } else {
                short
            }),
            405 => GeminiError::BadRequest("Method not allowed (HTTP 405) — the bot sent an unsupported operation.".into()),
            408 => GeminiError::Timeout,
            409 => GeminiError::BadRequest(if short.is_empty() {
                "Conflicting request (HTTP 409).".into()
            } else {
                short
            }),
            413 => GeminiError::PayloadTooLarge(if short.is_empty() {
                "Conversation is too large for the model.".into()
            } else {
                short
            }),
            429 => {
                if is_daily_quota(&details, &message) {
                    GeminiError::QuotaExhausted(if short.is_empty() {
                        "Daily quota exhausted.".into()
                    } else {
                        short
                    })
                } else {
                    GeminiError::RateLimited {
                        retry_after: retry_delay(&details).or(retry_after_header),
                    }
                }
            }
            500 | 502 | 503 | 504 => GeminiError::Unavailable {
                status,
                detail: if short.is_empty() {
                    match status {
                        500 => "Internal error at Google.".into(),
                        502 => "Bad gateway from Google.".into(),
                        503 => "Google is overloaded.".into(),
                        _ => "Gateway timeout from Google.".into(),
                    }
                } else {
                    short
                },
            },
            s if s >= 500 => GeminiError::Unavailable {
                status: s,
                detail: short,
            },
            s if (400..500).contains(&s) => GeminiError::BadRequest(format!(
                "HTTP {s}: {}",
                if short.is_empty() {
                    "the request was rejected"
                } else {
                    &short
                }
            )),
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
