use std::{env, str::FromStr};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Runtime configuration loaded from environment variables.
/// See `.env.example` for every option and its default.
#[derive(Clone)]
pub struct Config {
    pub discord_token: String,
    pub gemini_api_key: String,
    pub owner_id: u64,
    pub master_key: String,
    pub database_url: String,
    pub command_prefix: String,
    pub gemini_model: String,
    pub max_iterations: usize,
    pub temperature: f32,
    pub history_limit: usize,
    pub rate_limit_secs: u64,
    pub turn_timeout_secs: u64,
    pub max_tool_output_chars: usize,
    pub max_concurrent_turns: usize,
    pub max_concurrent_per_key: usize,
    pub queue_timeout_secs: u64,
    pub gemini_max_retries: u32,
    pub gemini_request_timeout_secs: u64,
    pub allow_agent_role_name: String,
    pub byok_session_secs: u64,
    pub error_log_channel_id: Option<u64>,
    pub register_slash_commands: bool,
    pub dev_guild_id: Option<u64>,
    pub resume_enabled: bool,
    pub resume_max_age_secs: i64,
    pub resume_max_attempts: i64,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("owner_id", &self.owner_id)
            .field("database_url", &self.database_url)
            .field("command_prefix", &self.command_prefix)
            .field("gemini_model", &self.gemini_model)
            .field("max_iterations", &self.max_iterations)
            .field("max_concurrent_turns", &self.max_concurrent_turns)
            .field("max_concurrent_per_key", &self.max_concurrent_per_key)
            .finish_non_exhaustive()
    }
}

fn required(name: &str) -> Result<String, BoxError> {
    let v = env::var(name).map_err(|_| format!("{name} is missing. Set it in your .env file."))?;
    let v = v.trim().to_string();
    if v.is_empty() {
        return Err(format!("{name} must not be empty.").into());
    }
    Ok(v)
}

fn parsed<T: FromStr>(name: &str, default: T) -> T {
    match env::var(name) {
        Ok(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                return default;
            }
            match trimmed.parse::<T>() {
                Ok(val) => val,
                Err(_) => {
                    tracing::warn!(
                        env_var = name,
                        value = %trimmed,
                        "invalid value, falling back to default"
                    );
                    default
                }
            }
        }
        Err(_) => default,
    }
}

fn parse_bool(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            match t.as_str() {
                "1" | "true" | "yes" | "y" | "on" => true,
                "0" | "false" | "no" | "n" | "off" => false,
                "" => default,
                _ => {
                    tracing::warn!(
                        env_var = name,
                        value = %v.trim(),
                        "invalid bool, expected true/false/1/0/yes/no/on/off; falling back to default"
                    );
                    default
                }
            }
        }
        Err(_) => default,
    }
}

fn optional_id(name: &str) -> Option<u64> {
    match env::var(name) {
        Ok(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                return None;
            }
            match trimmed.parse::<u64>() {
                Ok(id) if id != 0 => Some(id),
                Ok(_) => None,
                Err(_) => {
                    tracing::warn!(
                        env_var = name,
                        value = %trimmed,
                        "invalid snowflake ID, ignoring"
                    );
                    None
                }
            }
        }
        Err(_) => None,
    }
}

fn non_empty_or(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

impl Config {
    pub fn from_env() -> Result<Self, BoxError> {
        let discord_token = required("DISCORD_TOKEN")?;
        let gemini_api_key = required("GEMINI_API_KEY")?;
        let owner_id: u64 = required("OWNER_ID")?
            .parse()
            .map_err(|_| "OWNER_ID must be a valid 64-bit integer.".to_string())?;
        if owner_id == 0 {
            return Err("OWNER_ID must be a valid non-zero snowflake.".into());
        }
        let master_key = required("MASTER_KEY").map_err(|_| {
            "MASTER_KEY is missing. Generate one with `openssl rand -base64 32` and put it in .env \
             (it encrypts stored BYOK API keys — back it up, losing it makes stored keys unreadable)."
                .to_string()
        })?;

        let command_prefix = non_empty_or("COMMAND_PREFIX", "!");
        if command_prefix.chars().count() > 5 || command_prefix.contains(char::is_whitespace) {
            return Err("COMMAND_PREFIX must be 1-5 non-whitespace characters.".into());
        }

        Ok(Self {
            discord_token,
            gemini_api_key,
            owner_id,
            master_key,
            database_url: non_empty_or("DATABASE_URL", "sqlite://data/umbraix.db"),
            command_prefix,
            gemini_model: non_empty_or("GEMINI_MODEL", "gemini-flash-lite-latest"),
            max_iterations: parsed("MAX_ITERATIONS", 25usize).clamp(1, 256),
            temperature: {
                let t: f32 = parsed("AGENT_TEMPERATURE", 0.2f32);
                if t.is_finite() {
                    t.clamp(0.0, 2.0)
                } else {
                    0.2
                }
            },
            history_limit: parsed("HISTORY_LIMIT", 20usize).clamp(0, 100),
            rate_limit_secs: parsed("RATE_LIMIT_SECS", 3u64).clamp(1, 3600),
            turn_timeout_secs: parsed("TURN_TIMEOUT_SECS", 600u64).clamp(30, 1800),
            max_tool_output_chars: parsed("MAX_TOOL_OUTPUT_CHARS", 4000usize).clamp(500, 20_000),
            max_concurrent_turns: parsed("MAX_CONCURRENT_TURNS", 128usize).clamp(1, 2048),
            max_concurrent_per_key: parsed("MAX_CONCURRENT_PER_KEY", 8usize).clamp(1, 128),
            queue_timeout_secs: parsed("QUEUE_TIMEOUT_SECS", 60u64).clamp(5, 600),
            gemini_max_retries: parsed("GEMINI_MAX_RETRIES", 4u32).clamp(0, 8),
            gemini_request_timeout_secs: parsed("GEMINI_REQUEST_TIMEOUT_SECS", 60u64)
                .clamp(10, 300),
            allow_agent_role_name: non_empty_or("ALLOW_AGENT_ROLE_NAME", "allow_agent"),
            byok_session_secs: parsed("BYOK_SESSION_SECS", 300u64).clamp(60, 1800),
            error_log_channel_id: optional_id("ERROR_LOG_CHANNEL_ID"),
            register_slash_commands: parse_bool("REGISTER_SLASH_COMMANDS", true),
            dev_guild_id: optional_id("DEV_GUILD_ID"),
            resume_enabled: parse_bool("RESUME_ENABLED", true),
            resume_max_age_secs: parsed("RESUME_MAX_AGE_SECS", 1800i64).clamp(300, 86_400),
            resume_max_attempts: parsed("RESUME_MAX_ATTEMPTS", 2i64).clamp(1, 5),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    const BASE: &[(&str, &str)] = &[
        ("DISCORD_TOKEN", "x"),
        ("GEMINI_API_KEY", "y"),
        ("OWNER_ID", "123"),
        ("MASTER_KEY", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="),
    ];

    const OPTIONAL: &[&str] = &[
        "GEMINI_MODEL",
        "MAX_ITERATIONS",
        "AGENT_TEMPERATURE",
        "HISTORY_LIMIT",
        "RATE_LIMIT_SECS",
        "TURN_TIMEOUT_SECS",
        "MAX_TOOL_OUTPUT_CHARS",
        "MAX_CONCURRENT_TURNS",
        "MAX_CONCURRENT_PER_KEY",
        "QUEUE_TIMEOUT_SECS",
        "GEMINI_MAX_RETRIES",
        "GEMINI_REQUEST_TIMEOUT_SECS",
        "ALLOW_AGENT_ROLE_NAME",
        "BYOK_SESSION_SECS",
        "ERROR_LOG_CHANNEL_ID",
        "REGISTER_SLASH_COMMANDS",
        "DEV_GUILD_ID",
        "COMMAND_PREFIX",
        "DATABASE_URL",
    ];

    fn with_env(extra: &[(&str, &str)], f: impl FnOnce()) {
        let _guard = lock_env();
        let keys: Vec<&str> = BASE
            .iter()
            .chain(extra.iter())
            .map(|(k, _)| *k)
            .chain(OPTIONAL.iter().copied())
            .collect();
        let saved: Vec<(String, Option<String>)> = keys
            .iter()
            .map(|k| (k.to_string(), env::var(k).ok()))
            .collect();
        for k in OPTIONAL {
            unsafe { env::remove_var(k) };
        }
        for (k, v) in BASE.iter().chain(extra.iter()) {
            unsafe { env::set_var(k, v) };
        }
        f();
        for (k, old) in saved {
            match old {
                Some(v) => unsafe { env::set_var(&k, v) },
                None => unsafe { env::remove_var(&k) },
            }
        }
    }

    #[test]
    fn defaults_are_applied() {
        with_env(&[], || {
            let cfg = Config::from_env().unwrap();
            assert_eq!(cfg.owner_id, 123);
            assert_eq!(cfg.gemini_model, "gemini-flash-lite-latest");
            assert_eq!(cfg.max_iterations, 25);
            assert_eq!(cfg.turn_timeout_secs, 600);
            assert_eq!(cfg.max_tool_output_chars, 4000);
            assert_eq!(cfg.max_concurrent_turns, 128);
            assert_eq!(cfg.command_prefix, "!");
            assert_eq!(cfg.allow_agent_role_name, "allow_agent");
        });
    }

    #[test]
    fn max_iterations_is_clamped() {
        with_env(&[("MAX_ITERATIONS", "999")], || {
            assert_eq!(Config::from_env().unwrap().max_iterations, 256);
        });
    }

    #[test]
    fn max_iterations_allows_real_world_budgets() {
        with_env(&[("MAX_ITERATIONS", "200")], || {
            assert_eq!(Config::from_env().unwrap().max_iterations, 200);
        });
    }

    #[test]
    fn missing_master_key_is_explained() {
        with_env(&[], || {
            let saved = env::var("MASTER_KEY").ok();
            unsafe { env::remove_var("MASTER_KEY") };
            let err = Config::from_env().unwrap_err().to_string();
            assert!(err.contains("openssl rand"));
            if let Some(v) = saved {
                unsafe { env::set_var("MASTER_KEY", v) };
            }
        });
    }

    #[test]
    fn debug_does_not_leak_secrets() {
        with_env(&[], || {
            let cfg = Config::from_env().unwrap();
            let dbg = format!("{cfg:?}");
            assert!(!dbg.contains("AAAAAAAA"));
            assert!(!dbg.contains("discord_token"));
        });
    }
}
