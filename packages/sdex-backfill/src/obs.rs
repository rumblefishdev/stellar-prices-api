use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub fn init(verbose: bool) {
    let filter = if verbose {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("sdex_backfill=info"))
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("sdex_backfill=warn"))
    };

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(false).json())
        .with(filter)
        .init();
}
