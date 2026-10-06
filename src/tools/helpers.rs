use serde_json::Value;
use std::str::FromStr;
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker},
    Id,
};

/// Parse a Discord snowflake stored as a JSON string (what Gemini returns).
pub fn parse_channel_id(args: &Value, field: &str) -> Result<Id<ChannelMarker>, String> {
    let raw = args[field].as_str().unwrap_or_default().trim();
    if raw.is_empty() {
        return Err(format!("Missing required field '{field}'."));
    }
    Id::<ChannelMarker>::from_str(raw).map_err(|_| format!("Invalid channel ID '{raw}'."))
}

pub fn parse_guild_id(args: &Value, field: &str) -> Result<Id<GuildMarker>, String> {
    let raw = args[field].as_str().unwrap_or_default().trim();
    if raw.is_empty() {
        return Err(format!("Missing required field '{field}'."));
    }
    Id::<GuildMarker>::from_str(raw).map_err(|_| format!("Invalid guild ID '{raw}'."))
}

pub fn get_str<'a>(args: &'a Value, field: &str, default: &'a str) -> &'a str {
    args[field].as_str().unwrap_or(default).trim()
}

/// Clamp a requested purge count into Discord's 1..=100 window.
pub fn clamp_purge_count(args: &Value) -> u16 {
    args["count"].as_u64().unwrap_or(10).clamp(1, 100) as u16
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
}
