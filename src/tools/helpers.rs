use serde_json::Value;
use std::str::FromStr;
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, RoleMarker, UserMarker},
    Id,
};

/// Extract a raw snowflake string from a tool arg.
///
/// Gemini is instructed to send IDs as strings, but in practice it sometimes
/// sends JSON numbers (e.g. `{"channel_id": 123456789}`). Accept both, plus
/// Discord mention syntax, so one malformed arg doesn't burn a whole ReAct
/// turn.
fn raw_snowflake(args: &Value, field: &str) -> Result<String, String> {
    if let Some(s) = args[field].as_str() {
        let s = s.trim().to_string();
        if s.is_empty() {
            return Err(format!("Missing required field '{field}'."));
        }
        return Ok(s);
    }
    if let Some(n) = args[field].as_u64() {
        if n == 0 {
            return Err(format!("Missing required field '{field}'."));
        }
        return Ok(n.to_string());
    }
    if let Some(n) = args[field].as_i64() {
        if n <= 0 {
            return Err(format!("Missing required field '{field}'."));
        }
        return Ok(n.to_string());
    }
    Err(format!("Missing required field '{field}'."))
}

/// Parse a Discord snowflake (accepts raw IDs, JSON numbers, `<#123>` mentions).
pub fn parse_channel_id(args: &Value, field: &str) -> Result<Id<ChannelMarker>, String> {
    let raw = raw_snowflake(args, field)?;
    let raw = raw.trim();
    // Accept `<#123>` channel mentions as well as raw snowflakes.
    let digits = raw.trim_start_matches("<#").trim_end_matches('>').trim();
    Id::<ChannelMarker>::from_str(digits).map_err(|_| format!("Invalid channel ID '{raw}'."))
}

pub fn parse_user_id(args: &Value, field: &str) -> Result<Id<UserMarker>, String> {
    let raw = raw_snowflake(args, field)?;
    let raw = raw.trim();
    // Accept raw snowflakes and `<@123>` / `<@!123>` mentions.
    let digits = raw
        .trim_start_matches("<@")
        .trim_start_matches('!')
        .trim_end_matches('>');
    // Guard: a role mention `<@&123>` passed where a user was expected.
    let digits = digits.trim_start_matches('&');
    if digits.is_empty() {
        return Err(format!("Missing required field '{field}'."));
    }
    Id::<UserMarker>::from_str(digits).map_err(|_| format!("Invalid user ID '{raw}'."))
}

pub fn parse_role_id(args: &Value, field: &str) -> Result<Id<RoleMarker>, String> {
    let raw = raw_snowflake(args, field)?;
    let raw = raw.trim();
    // Accept raw snowflakes and `<@&123>` role mentions.
    let digits = raw.trim_start_matches("<@&").trim_end_matches('>');
    let digits = digits.trim();
    if digits.is_empty() {
        return Err(format!("Missing required field '{field}'."));
    }
    Id::<RoleMarker>::from_str(digits).map_err(|_| format!("Invalid role ID '{raw}'."))
}

pub fn parse_guild_id(args: &Value, field: &str) -> Result<Id<GuildMarker>, String> {
    let raw = raw_snowflake(args, field)?;
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(format!("Missing required field '{field}'."));
    }
    Id::<GuildMarker>::from_str(raw).map_err(|_| format!("Invalid guild ID '{raw}'."))
}

pub fn get_str<'a>(args: &'a Value, field: &str, default: &'a str) -> &'a str {
    args[field].as_str().unwrap_or(default).trim()
}

/// Clamp a requested purge count into Discord's 1..=100 window.
/// Accepts numbers or numeric strings (the model sends both).
pub fn clamp_purge_count(args: &Value) -> u16 {
    let n = if let Some(n) = args["count"].as_u64() {
        n
    } else if let Some(s) = args["count"].as_str() {
        s.trim().parse::<u64>().unwrap_or(10)
    } else {
        10
    };
    n.clamp(1, 100) as u16
}

/// Clamp a generic bounded integer arg that may arrive as number or string.
pub fn clamp_int_arg(args: &Value, field: &str, default: u64, min: u64, max: u64) -> u64 {
    let n = if let Some(n) = args[field].as_u64() {
        n
    } else if let Some(s) = args[field].as_str() {
        s.trim().parse::<u64>().unwrap_or(default)
    } else {
        default
    };
    n.clamp(min, max)
}

/// Truncate tool output so one huge listing can't blow the model context.
pub fn truncate_output(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!("{kept}\n…(truncated to {max_chars} chars)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_missing_channel() {
        assert!(parse_channel_id(&json!({}), "channel_id").is_err());
    }

    #[test]
    fn rejects_garbage_channel() {
        assert!(parse_channel_id(&json!({"channel_id": "abc"}), "channel_id").is_err());
    }

    #[test]
    fn clamps_purge_count() {
        assert_eq!(clamp_purge_count(&json!({})), 10);
        assert_eq!(clamp_purge_count(&json!({"count": 0})), 1);
        assert_eq!(clamp_purge_count(&json!({"count": 500})), 100);
    }

    #[test]
    fn accepts_mentions_for_user_and_role() {
        assert!(parse_user_id(&json!({"user_id": "<@123456789012345678>"}), "user_id").is_ok());
        assert!(parse_user_id(&json!({"user_id": "<@!123456789012345678>"}), "user_id").is_ok());
        assert!(parse_role_id(&json!({"role_id": "<@&123456789012345678>"}), "role_id").is_ok());
        assert!(parse_user_id(&json!({"user_id": "not-an-id"}), "user_id").is_err());
        assert!(parse_role_id(&json!({}), "role_id").is_err());
    }

    #[test]
    fn accepts_numeric_ids_and_channel_mentions() {
        assert!(
            parse_channel_id(&json!({"channel_id": 123456789012345678u64}), "channel_id").is_ok()
        );
        assert!(parse_channel_id(
            &json!({"channel_id": "<#123456789012345678>"}),
            "channel_id"
        )
        .is_ok());
        assert!(parse_user_id(&json!({"user_id": 123456789012345678u64}), "user_id").is_ok());
        assert!(parse_role_id(&json!({"role_id": 123456789012345678u64}), "role_id").is_ok());
    }
}
