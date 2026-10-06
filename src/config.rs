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
/// - `MAX_ITERATIONS` (default: 10, clamped 1..=25)
/// - `AGENT_TEMPERATURE` (default: 0.2)
/// - `HISTORY_LIMIT` (default: 20, clamped 0..=100)
/// - `RATE_LIMIT_SECS` (default: 3)
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

        let max_iterations = env::var("MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(10)
            .clamp(1, 25);

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
            .unwrap_or(3);

        Ok(Self {
            discord_token,
            gemini_api_key,
            owner_id,
            gemini_model,
            max_iterations,
            temperature,
            history_limit,
            rate_limit_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env(vars: &[(&str, &str)], f: impl FnOnce()) {
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
                }
                let cfg = Config::from_env().unwrap();
                assert_eq!(cfg.owner_id, 123);
                assert_eq!(cfg.gemini_model, "gemini-flash-lite-latest");
                assert_eq!(cfg.max_iterations, 10);
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
                assert_eq!(cfg.max_iterations, 25);
            },
        );
    }
}
