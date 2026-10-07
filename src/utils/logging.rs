use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

/// Initialize structured logging.
///
/// - `RUST_LOG`   filter (default `info,sqlx=warn`)
/// - `LOG_FORMAT` `pretty` (default) or `json` for the console
/// - `LOG_DIR`    when set, also writes daily-rotated JSON logs there
///
/// Returns a guard that must be kept alive so buffered file logs flush on exit.
pub fn init() -> Option<WorkerGuard> {
    let filter = || match EnvFilter::try_from_default_env() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("warning: invalid RUST_LOG ({e}), using default 'info,sqlx=warn,twilight_gateway=info'");
            EnvFilter::new("info,sqlx=warn,twilight_gateway=info")
        }
    };
    let json = std::env::var("LOG_FORMAT")
        .map(|v| v.trim().eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let console = if json {
        fmt::layer()
            .json()
            .with_current_span(true)
            .with_target(true)
            .with_filter(filter())
            .boxed()
    } else {
        fmt::layer()
            .with_target(false)
            .with_filter(filter())
            .boxed()
    };

    let (file_layer, guard) = match std::env::var("LOG_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let dir = dir.trim().to_string();
            // Validate early: fail loudly instead of silently losing file logs.
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("warning: LOG_DIR={dir} not writable ({e}), file logging disabled");
                (None, None)
            } else {
                let appender = tracing_appender::rolling::daily(&dir, "umbraix.log");
                let (writer, guard) = tracing_appender::non_blocking(appender);
                let layer = fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_writer(writer)
                    .with_ansi(false)
                    .with_filter(filter())
                    .boxed();
                (Some(layer), Some(guard))
            }
        }
        _ => (None, None),
    };

    let _ = tracing_subscriber::registry()
        .with(console)
        .with(file_layer)
        .try_init();
    guard
}
