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
    let _ = rustls::crypto::ring::default_provider().install_default();
    dotenvy::dotenv().ok();
    let _log_guard = logging::init();

    if let Err(e) = run().await {
        tracing::error!(error = %e, "{} failed to start", brand::NAME);
        return Err(e);
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
