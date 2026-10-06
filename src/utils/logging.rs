/// Initialize structured logging from `RUST_LOG`.
///
/// Uses `try_init` so unit tests that initialize logging more than once
/// do not panic.
pub fn init() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
