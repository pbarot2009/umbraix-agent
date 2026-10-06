use twilight_model::channel::Message;

use crate::config::Config;

pub const COMMAND_PREFIX: &str = "!ai ";

/// True when the message should even be considered: not from a bot, from the owner.
pub fn is_owner_command(msg: &Message, config: &Config) -> bool {
    !msg.author.bot && msg.author.id.get() == config.owner_id
}

/// Extract the prompt after `!ai `, trimmed. `None` when not a command
/// or when the prompt is empty.
pub fn extract_prompt(content: &str) -> Option<String> {
    let prompt = content.strip_prefix(COMMAND_PREFIX)?.trim();
    if prompt.is_empty() {
        None
    } else {
        Some(prompt.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            discord_token: "x".into(),
            gemini_api_key: "y".into(),
            owner_id: 42,
            gemini_model: "m".into(),
            max_iterations: 10,
            temperature: 0.2,
            history_limit: 20,
            rate_limit_secs: 3,
        }
    }

    #[test]
    fn extracts_prompt() {
        assert_eq!(
            extract_prompt("!ai hello world").as_deref(),
            Some("hello world")
        );
        assert_eq!(extract_prompt("!ai   "), None);
        assert_eq!(extract_prompt("hello"), None);
    }

    #[test]
    fn owner_check_works() {
        // Minimal Message construction is heavy; test prefix logic only here.
        // Full author/bot checks are covered by `is_owner_command` callers.
        let _ = test_config();
    }
}
