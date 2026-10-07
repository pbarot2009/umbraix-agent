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

/// Classify a raw Discord / twilight error string into a stable code.
///
/// Uses explicit Discord JSON error-code matching with word boundaries to
/// avoid false positives (e.g. an ID containing "429" must not be treated
/// as rate-limited). Falls back to case-insensitive phrase matching.
pub fn discord_code(raw: &str) -> &'static str {
    let lc = raw.to_lowercase();
    // Helper: match a numeric Discord API code as a standalone token,
    // not as a substring of a larger number / snowflake.
    let has_code = |code: &str| {
        // Fast path: exact substring must exist at all.
        if !raw.contains(code) {
            return false;
        }
        // Verify token boundaries around each occurrence.
        let mut start = 0;
        while let Some(pos) = raw[start..].find(code) {
            let s = start + pos;
            let e = s + code.len();
            let before_ok = s == 0 || !raw.as_bytes()[s - 1].is_ascii_digit();
            let after_ok = e == raw.len() || !raw.as_bytes()[e].is_ascii_digit();
            if before_ok && after_ok {
                return true;
            }
            start = s + 1;
            if start >= raw.len() {
                break;
            }
        }
        false
    };

    if has_code("50013") || lc.contains("missing permissions") {
        "DISCORD_MISSING_PERMISSIONS"
    } else if has_code("50001") || lc.contains("missing access") {
        "DISCORD_MISSING_ACCESS"
    } else if has_code("50007") {
        "DISCORD_CANNOT_DM"
    } else if has_code("10003") {
        "DISCORD_UNKNOWN_CHANNEL"
    } else if has_code("10004") {
        "DISCORD_UNKNOWN_GUILD"
    } else if has_code("10007") || has_code("10013") {
        "DISCORD_UNKNOWN_USER"
    } else if has_code("10011") {
        "DISCORD_UNKNOWN_ROLE"
    } else if has_code("10008") {
        "DISCORD_UNKNOWN_MESSAGE"
    } else if has_code("10062") {
        "DISCORD_UNKNOWN_INTERACTION"
    } else if has_code("30001")
        || has_code("30005")
        || has_code("30013")
        || has_code("30016")
        || lc.contains("maximum number")
    {
        "DISCORD_LIMIT_REACHED"
    } else if has_code("50035") {
        "DISCORD_INVALID_FORM"
    } else if has_code("50024") {
        "DISCORD_WRONG_CHANNEL_TYPE"
    } else if has_code("50028") {
        "DISCORD_INVALID_ROLE"
    } else if has_code("429") || lc.contains("rate limit") {
        "DISCORD_RATE_LIMITED"
    } else if has_code("401") || lc.contains("unauthorized") {
        "DISCORD_UNAUTHORIZED"
    } else if has_code("403") || lc.contains("forbidden") {
        "DISCORD_FORBIDDEN"
    } else if has_code("404") || lc.contains("not found") {
        "DISCORD_NOT_FOUND"
    } else if has_code("500") || has_code("502") || has_code("503") || has_code("504") {
        "DISCORD_UNAVAILABLE"
    } else if lc.contains("hierarchy") || lc.contains("higher role") {
        "DISCORD_HIERARCHY"
    } else {
        "DISCORD_API"
    }
}

/// Human hint for a raw Discord error. Every common code gets an actionable
/// message — previously all Discord failures shared one generic line.
pub fn discord_hint(raw: &str) -> String {
    let detail = crate::utils::truncate_chars(raw, 250);
    match discord_code(raw) {
        "DISCORD_MISSING_PERMISSIONS" => format!(
            "Discord says I'm missing permissions (code 50013). Give me the permission the action needs and make sure my role is ABOVE the target roles/members in Server Settings → Roles. Detail: {detail}"
        ),
        "DISCORD_HIERARCHY" | "DISCORD_FORBIDDEN" => format!(
            "Discord blocked this for hierarchy/position reasons — I can't act on targets at or above my own highest role. Drag my role higher in Server Settings → Roles, then retry. Detail: {detail}"
        ),
        "DISCORD_MISSING_ACCESS" => format!(
            "I can't access that channel (code 50001). Give me View Channel there (and Send Messages to speak). Detail: {detail}"
        ),
        "DISCORD_CANNOT_DM" => "I can't DM that user — their DMs are closed. Ask them to enable DMs or use a slash command instead.".to_string(),
        "DISCORD_UNKNOWN_CHANNEL" => format!(
            "That channel doesn't exist (or I can't see it). Run `list_channels` to re-ground and use a fresh ID. Detail: {detail}"
        ),
        "DISCORD_UNKNOWN_GUILD" => "I can't see that server anymore — was I removed?".to_string(),
        "DISCORD_UNKNOWN_USER" => format!(
            "That user isn't in this server (or the ID is wrong). Use `search_members` to find the right ID; `ban_member` by ID still works for users who left. Detail: {detail}"
        ),
        "DISCORD_UNKNOWN_ROLE" => format!(
            "That role doesn't exist. Run `list_roles` to re-ground and use a fresh ID. Detail: {detail}"
        ),
        "DISCORD_UNKNOWN_MESSAGE" => "That message no longer exists (deleted or too old).".to_string(),
        "DISCORD_UNKNOWN_INTERACTION" => "That button/form expired (interactions last 15 min). Run the command again.".to_string(),
        "DISCORD_LIMIT_REACHED" => format!(
            "Discord's server limit was hit (too many channels/roles/emojis). Delete something first, then retry. Detail: {detail}"
        ),
        "DISCORD_INVALID_FORM" => format!(
            "Discord rejected the values (code 50035 — e.g. name/topic too long or bad characters). Shorten/fix the text and retry. Detail: {detail}"
        ),
        "DISCORD_WRONG_CHANNEL_TYPE" => "That action doesn't work on this channel type (e.g. topic on a voice channel). Pick a text/announcement channel.".to_string(),
        "DISCORD_INVALID_ROLE" => "That role can't be used this way (managed, @everyone, or invalid). Pick a normal custom role.".to_string(),
        "DISCORD_RATE_LIMITED" => "Discord is rate-limiting the bot right now. Wait ~1 minute and retry.".to_string(),
        "DISCORD_UNAUTHORIZED" => "My Discord token is invalid — the bot owner must reset DISCORD_TOKEN.".to_string(),
        "DISCORD_NOT_FOUND" => format!(
            "Discord couldn't find that item (404). IDs may be stale — re-ground with `list_channels`/`list_roles`/`search_members`. Detail: {detail}"
        ),
        "DISCORD_UNAVAILABLE" => "Discord is having an outage right now. I kept your request safe — try again in a minute.".to_string(),
        _ => format!(
            "Make sure I have the needed permissions and my role is above the roles I manage. Detail: {detail}"
        ),
    }
}

/// Friendly mapping for a tool execution failure (`Failed to execute X: ...`).
/// Gives the MODEL actionable guidance instead of a raw HTTP dump.
pub fn friendly_tool_error(tool: &str, raw: &str) -> String {
    let code = discord_code(raw);
    let hint = discord_hint(raw);
    // Keep it short for model context: tool name + code + hint, capped.
    let safe_tool: String = tool.chars().take(64).collect();
    let msg = format!("Failed to execute {safe_tool} [{code}]: {hint}");
    crate::utils::truncate_chars(&msg, 800)
}

impl BotError {
    pub fn code(&self) -> &'static str {
        match self {
            BotError::Gemini(e) => e.code(),
            BotError::Discord(d) => discord_code(d),
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
                GeminiError::PayloadTooLarge(_)
                    | GeminiError::Unavailable { .. }
                    | GeminiError::ServiceDisabled(_)
            ),
            BotError::Discord(d) => matches!(
                discord_code(d),
                "DISCORD_UNAUTHORIZED"
                    | "DISCORD_UNAVAILABLE"
                    | "DISCORD_API"
                    | "DISCORD_RATE_LIMITED"
            ),
            BotError::Store(_) | BotError::Internal(_) => true,
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
                GeminiError::ServiceDisabled(_) => "Gemini API disabled",
                GeminiError::RegionUnsupported(_) => "Gemini unavailable in region",
                GeminiError::PayloadTooLarge(_) => "Request too large",
                GeminiError::Unavailable { .. } => "Gemini is having trouble",
                GeminiError::Timeout => "Gemini took too long",
                GeminiError::Network(_) => "Couldn't reach Gemini",
                GeminiError::Blocked(_) => "Response blocked by safety filters",
                GeminiError::Malformed(_) => "Unexpected Gemini response",
                GeminiError::KeyCoolingDown { .. } => "API key temporarily paused",
            },
            BotError::Discord(d) => match discord_code(d) {
                "DISCORD_MISSING_PERMISSIONS" => "I'm missing Discord permissions",
                "DISCORD_MISSING_ACCESS" => "I can't access that channel",
                "DISCORD_CANNOT_DM" => "Can't send a DM",
                "DISCORD_UNKNOWN_CHANNEL" => "Channel not found",
                "DISCORD_UNKNOWN_GUILD" => "Server not found",
                "DISCORD_UNKNOWN_USER" => "User not found",
                "DISCORD_UNKNOWN_ROLE" => "Role not found",
                "DISCORD_UNKNOWN_MESSAGE" => "Message not found",
                "DISCORD_UNKNOWN_INTERACTION" => "Interaction expired",
                "DISCORD_LIMIT_REACHED" => "Discord server limit reached",
                "DISCORD_INVALID_FORM" => "Discord rejected the values",
                "DISCORD_WRONG_CHANNEL_TYPE" => "Wrong channel type",
                "DISCORD_INVALID_ROLE" => "Invalid role",
                "DISCORD_HIERARCHY" => "Role hierarchy blocks this",
                "DISCORD_FORBIDDEN" => "Discord forbids this",
                "DISCORD_NOT_FOUND" => "Not found on Discord",
                "DISCORD_UNAUTHORIZED" => "Discord token invalid",
                "DISCORD_RATE_LIMITED" => "Discord is rate limiting me",
                "DISCORD_UNAVAILABLE" => "Discord is having trouble",
                _ => "Discord rejected an action",
            },
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
                    "Billing may be disabled for this Google project ({d}). Enable billing or use a key from a billed project. {fix_key}"
                ),
                GeminiError::ServiceDisabled(d) => format!(
                    "The Generative Language API is disabled for this Google project ({d}). Enable it at https://console.cloud.google.com/apis/library/generativelanguage.googleapis.com then try again. {fix_key}"
                ),
                GeminiError::RegionUnsupported(d) => format!(
                    "Gemini isn't available for this model in your region ({d}). Try a different model (e.g. `{prefix}config model gemini-2.5-flash`) or a key from a supported region."
                ),
                GeminiError::PayloadTooLarge(d) => format!(
                    "The request is too large for the model (context/token limit: {d}). Run `{prefix}clear` to reset the conversation, shorten the request, and try again."
                ),
                GeminiError::Unavailable { status, detail } => format!(
                    "I retried automatically but Gemini is still struggling (HTTP {status}: {detail}). Please try again in a minute."
                ),
                GeminiError::Timeout | GeminiError::Network(_) => {
                    "I retried automatically but couldn't reach Gemini. Please try again in a minute.".to_string()
                }
                GeminiError::Blocked(r) => {
                    format!("Gemini refused to answer ({r}). Rephrase the request more neutrally.")
                }
                GeminiError::Malformed(_) => "Please try again; if it keeps happening, report the reference ID.".to_string(),
                GeminiError::KeyCoolingDown { remaining, reason } => format!(
                    "After a '{reason}' error the key is paused for {}s to avoid wasted calls. {fix_key}",
                    remaining.as_secs().max(1)
                ),
            },
            BotError::Discord(d) => discord_hint(d),
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
