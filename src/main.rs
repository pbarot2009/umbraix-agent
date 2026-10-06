use discord_gemini_agent::{config::Config, context::Context, discord, utils::logging};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Install Rustls CryptoProvider (prevents process-level Rustls panic).
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 2. Load `.env` (ignored when variables are exported directly).
    dotenvy::dotenv().ok();

    // 3. Structured logging from `RUST_LOG` (defaults to `info`).
    logging::init();

    // 4. Validate configuration from the environment.
    let config = Config::from_env()?;

    // 5. Shared state + gateway loop (never returns under normal operation).
    let ctx = Context::new(config);
    discord::run(ctx).await
}
