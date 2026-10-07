use umbraix_agent::{
    brand,
    config::Config,
    context::Context,
    discord,
    storage::{Store, Vault},
    utils::logging,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Err(e) = rustls::crypto::ring::default_provider().install_default() {
        eprintln!("rustls default provider install failed (already installed?): {e:?}");
    }
    if let Err(e) = dotenvy::dotenv() {
        // Missing .env is fine (env may come from shell); malformed is not.
        if e.not_found() {
            tracing::debug!(".env not found, using process environment");
        } else {
            eprintln!("warning: failed to parse .env: {e}");
        }
    }
    let _log_guard = logging::init();

    if let Err(e) = run().await {
        // Log once here; return Ok to avoid double-reporting via Debug print.
        tracing::error!(error = %e, "{} failed to start", brand::NAME);
        std::process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_env()?;
    tracing::info!(
        ?config,
        "{} v{} configuration loaded",
        brand::NAME,
        brand::VERSION
    );

    let vault = Vault::from_base64(&config.master_key)?;
    let store = Store::connect(&config.database_url, vault).await?;

    let ctx = Context::new(config, store);
    discord::run(ctx).await
}
