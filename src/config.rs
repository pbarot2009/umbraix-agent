use std::env;

/// Runtime configuration loaded from environment variables.
///
/// Required:
/// - `DISCORD_TOKEN`
/// - `GEMINI_API_KEY`
/// - `OWNER_ID` (u64 snowflake)
///
/// Optional (with defaults):
/// - `GEMINI_MODEL` (default: `gemini-flash-lite-latest`)
/// - `MAX_ITERATIONS` (default: 10, clamped 1..=100)
/// - `AGENT_TEMPERATURE` (default: 0.2)
/// - `HISTORY_LIMIT` (default: 20, clamped 0..=100)
/// - `RATE_LIMIT_SECS` (default: 3)
/// - `TURN_TIMEOUT_SECS` (default: 300, clamped 30..=1800)
/// - `MAX_TOOL_OUTPUT_CHARS` (default: 4000, clamped 500..=20000)
#[derive(Debug, Clone)]
pub struct Config {
    pub discord_token: String,
    pub gemini_api_key: String,
    pub owner_id: u64,
    pub gemini_model: String,
    pub max_iterations: usize,
    pub temperature: f32,
    pub history_limit: usize,
    pub rate_limit_secs: u64,
    pub turn_timeout_secs: u64,
    pub max_tool_output_chars: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let discord_token = env::var("DISCORD_TOKEN").map_err(|_| {
            "DISCORD_TOKEN is missing. Please set it in your .env file.".to_string()
        })?;
        let gemini_api_key = env::var("GEMINI_API_KEY").map_err(|_| {
            "GEMINI_API_KEY is missing. Please set it in your .env file.".to_string()
        })?;
        let owner_id_str = env::var("OWNER_ID")
            .map_err(|_| "OWNER_ID is missing. Please set it in your .env file.".to_string())?;
        let owner_id: u64 = owner_id_str
            .trim()
            .parse()
            .map_err(|_| "OWNER_ID must be a valid 64-bit integer.".to_string())?;

        if discord_token.trim().is_empty() {
            return Err("DISCORD_TOKEN must not be empty.".into());
        }
        if gemini_api_key.trim().is_empty() {
            return Err("GEMINI_API_KEY must not be empty.".into());
        }

        let gemini_model =
            env::var("GEMINI_MODEL").unwrap_or_else(|_| "gemini-flash-lite-latest".to_string());
        let gemini_model = gemini_model.trim().to_string();
        if gemini_model.is_empty() {
            return Err("GEMINI_MODEL must not be empty.".into());
        }

        // Real-world tasks (bulk moderation, channel setup, role grants)
        // routinely need 30-60 tool steps. The old 25-step hard cap forced
        // premature "ran out of steps" failures, so allow up to 100 steps
        // with loop-detection + turn timeout as the real safety rails
        // (instead of an artificially low step count).
        let max_iterations = env::var("MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(10)
            .clamp(1, 100);

        let temperature = env::var("AGENT_TEMPERATURE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.2)
            .clamp(0.0, 2.0);

        let history_limit = env::var("HISTORY_LIMIT")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(20)
            .clamp(0, 100);

        let rate_limit_secs = env::var("RATE_LIMIT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(3)
            .clamp(1, 3600);

        if owner_id == 0 {
            return Err("OWNER_ID must be a valid non-zero snowflake.".into());
        }

        let turn_timeout_secs = env::var("TURN_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(30, 1800);

        let max_tool_output_chars = env::var("MAX_TOOL_OUTPUT_CHARS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(4000)
            .clamp(500, 20_000);

        Ok(Self {
            discord_token,
            gemini_api_key,
            owner_id,
            gemini_model,
            max_iterations,
            temperature,
            history_limit,
            rate_limit_secs,
            turn_timeout_secs,
            max_tool_output_chars,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    // Env vars are process-global; without a lock these tests race each other
    // when cargo runs them in parallel (one test's MAX_ITERATIONS leaks into
    // another's defaults check).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_env(vars: &[(&str, &str)], f: impl FnOnce()) {
        let _guard = lock_env();
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), env::var(k).ok()))
            .collect();
        for (k, v) in vars {
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
        with_env(
            &[
                ("DISCORD_TOKEN", "x"),
                ("GEMINI_API_KEY", "y"),
                ("OWNER_ID", "123"),
            ],
            || {
                unsafe {
                    env::remove_var("GEMINI_MODEL");
                    env::remove_var("MAX_ITERATIONS");
                    env::remove_var("AGENT_TEMPERATURE");
                    env::remove_var("HISTORY_LIMIT");
                    env::remove_var("RATE_LIMIT_SECS");
                    env::remove_var("TURN_TIMEOUT_SECS");
                    env::remove_var("MAX_TOOL_OUTPUT_CHARS");
                }
                let cfg = Config::from_env().unwrap();
                assert_eq!(cfg.owner_id, 123);
                assert_eq!(cfg.gemini_model, "gemini-flash-lite-latest");
                assert_eq!(cfg.max_iterations, 10);
                assert_eq!(cfg.turn_timeout_secs, 300);
                assert_eq!(cfg.max_tool_output_chars, 4000);
            },
        );
    }

    #[test]
    fn max_iterations_is_clamped() {
        with_env(
            &[
                ("DISCORD_TOKEN", "x"),
                ("GEMINI_API_KEY", "y"),
                ("OWNER_ID", "123"),
                ("MAX_ITERATIONS", "999"),
            ],
            || {
                let cfg = Config::from_env().unwrap();
                assert_eq!(cfg.max_iterations, 100);
            },
        );
    }

    #[test]
    fn max_iterations_allows_real_world_budgets() {
        with_env(
            &[
                ("DISCORD_TOKEN", "x"),
                ("GEMINI_API_KEY", "y"),
                ("OWNER_ID", "123"),
                ("MAX_ITERATIONS", "60"),
            ],
            || {
                let cfg = Config::from_env().unwrap();
                assert_eq!(cfg.max_iterations, 60);
            },
        );
    }
}
