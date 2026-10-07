use crate::{access::KeySource, gemini::GeminiError, storage::StoreError};

/// Top-level error for a user request. Each variant maps to a stable code,
/// a friendly title and an actionable hint shown in Discord, while the full
/// detail goes to logs under a short reference ID.
#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error(transparent)]
    Gemini(#[from] GeminiError),
    #[error("Discord API error: {0}")]
    Discord(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("request timed out after {0}s")]
    Timeout(u64),
    #[error("{0}")]
    User(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<twilight_http::Error> for BotError {
    fn from(e: twilight_http::Error) -> Self {
        BotError::Discord(e.to_string())
    }
}

impl From<twilight_http::response::DeserializeBodyError> for BotError {
    fn from(e: twilight_http::response::DeserializeBodyError) -> Self {
        BotError::Discord(format!("bad response body: {e}"))
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for BotError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        BotError::Internal(e.to_string())
    }
}

impl BotError {
    pub fn code(&self) -> &'static str {
        match self {
            BotError::Gemini(e) => e.code(),
            BotError::Discord(_) => "DISCORD_API",
            BotError::Store(_) => "STORAGE",
            BotError::Timeout(_) => "TIMEOUT",
            BotError::User(_) => "USER",
            BotError::Internal(_) => "INTERNAL",
        }
    }

    /// Errors worth alerting the bot operator about (not user mistakes).
    pub fn is_operator_relevant(&self) -> bool {
        match self {
            BotError::Gemini(e) => matches!(
                e,
                GeminiError::Malformed(_)
                    | GeminiError::BadRequest(_)
                    | GeminiError::Unavailable { .. }
            ),
            BotError::Discord(_) | BotError::Store(_) | BotError::Internal(_) => true,
            BotError::Timeout(_) | BotError::User(_) => false,
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            BotError::Gemini(e) => match e {
                GeminiError::InvalidKey => "API key rejected",
                GeminiError::PermissionDenied(_) => "API key not permitted",
                GeminiError::RateLimited { .. } => "Gemini is rate limiting this key",
                GeminiError::QuotaExhausted(_) => "Gemini quota exhausted",
                GeminiError::ModelNotFound(_) => "Model unavailable",
                GeminiError::BadRequest(_) => "Gemini rejected the request",
                GeminiError::Precondition(_) => "Gemini billing problem",
                GeminiError::Unavailable { .. } => "Gemini is having trouble",
                GeminiError::Timeout => "Gemini took too long",
                GeminiError::Network(_) => "Couldn't reach Gemini",
                GeminiError::Blocked(_) => "Response blocked by safety filters",
                GeminiError::Malformed(_) => "Unexpected Gemini response",
                GeminiError::KeyCoolingDown { .. } => "API key temporarily paused",
            },
            BotError::Discord(_) => "Discord rejected an action",
            BotError::Store(_) => "Storage problem",
            BotError::Timeout(_) => "Request timed out",
            BotError::User(_) => "Can't do that",
            BotError::Internal(_) => "Something broke on our side",
        }
    }

    /// Actionable guidance tailored to whose key was in use.
    pub fn hint(&self, source: Option<KeySource>, prefix: &str) -> String {
        let fix_key = match source {
            Some(KeySource::User) => format!(
                "Your personal key needs attention — run `{prefix}byok-user` (or `/byok-user`) to set a new one."
            ),
            Some(KeySource::Server) => format!(
                "This server's key needs attention — the server owner should run `{prefix}byok-server`."
            ),
            Some(KeySource::Owner) => "The bot owner's key needs attention (check GEMINI_API_KEY).".to_string(),
            None => format!("Check the key with `{prefix}byok-status`."),
        };
        match self {
            BotError::Gemini(e) => match e {
                GeminiError::InvalidKey => format!("Google says the key is invalid, revoked or leaked. {fix_key}"),
                GeminiError::PermissionDenied(d) => {
                    format!("The key can't access the Generative Language API ({d}). Enable it in Google AI Studio. {fix_key}")
                }
                GeminiError::RateLimited { .. } => {
                    "Too many requests per minute on this key's Google project. Wait a moment and try again; \
                     limits apply per project, so a second key from the same project won't help."
                        .to_string()
                }
                GeminiError::QuotaExhausted(_) => format!(
                    "The daily quota is used up (resets at midnight Pacific). Upgrade the Google AI Studio tier or use another project's key. {fix_key}"
                ),
                GeminiError::ModelNotFound(_) => format!(
                    "The configured model isn't available for this key. A server owner can change it with `{prefix}config model <name>`."
                ),
                GeminiError::BadRequest(d) => format!(
                    "Try `{prefix}clear` to reset the conversation and rephrase. Detail: {d}"
                ),
                GeminiError::Precondition(d) => format!(
                    "Billing may be disabled or the region unsupported ({d}). {fix_key}"
                ),
                GeminiError::Unavailable { .. } | GeminiError::Timeout | GeminiError::Network(_) => {
                    "I retried automatically but Gemini is still struggling. Please try again in a minute.".to_string()
                }
                GeminiError::Blocked(r) => {
                    format!("Gemini refused to answer ({r}). Rephrase the request.")
                }
                GeminiError::Malformed(_) => "Please try again; if it keeps happening, report the reference ID.".to_string(),
                GeminiError::KeyCoolingDown { remaining, reason } => format!(
                    "After a '{reason}' error the key is paused for {}s to avoid wasted calls. {fix_key}",
                    remaining.as_secs().max(1)
                ),
            },
            BotError::Discord(d) => format!(
                "Make sure I have the needed permissions and my role is above the roles I manage. Detail: {}",
                crate::utils::truncate_chars(d, 300)
            ),
            BotError::Store(_) => "The database is unavailable right now. Please try again shortly.".to_string(),
            BotError::Timeout(secs) => format!(
                "Stopped after {secs}s to avoid hanging. Any actions already taken were applied — ask me to continue."
            ),
            BotError::User(msg) => msg.clone(),
            BotError::Internal(_) => "Please try again; if it keeps happening, report the reference ID.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_user_key_points_to_byok_user() {
        let e = BotError::Gemini(GeminiError::InvalidKey);
        let hint = e.hint(Some(KeySource::User), "!");
        assert!(hint.contains("!byok-user"));
        assert_eq!(e.code(), "GEMINI_INVALID_KEY");
        assert!(!e.is_operator_relevant());
    }

    #[test]
    fn server_key_points_to_owner() {
        let e = BotError::Gemini(GeminiError::QuotaExhausted("x".into()));
        assert!(e
            .hint(Some(KeySource::Server), "!")
            .contains("!byok-server"));
    }
}
