use anyhow::Result;
use tracing::info;

mod classifier;
mod config;
mod context;
mod mapper;
mod platform;

// current-thread runtime: одна нить async-event-loop. Нам этого достаточно
// (event-driven, мало CPU-bound работы), и это убирает Send-требование на future,
// что позволяет держать non-Send типы (например, `xkb::State`) прямо в state.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    init_tracing();
    info!("MaTea v{} starting", env!("CARGO_PKG_VERSION"));

    let cfg = config::load()?;
    info!(?cfg, "config loaded");

    let platform = platform::current().await?;
    info!("platform: {}", platform.name());
    platform.run().await?;
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("matea=debug,info"));
    fmt().with_env_filter(filter).with_target(false).init();
}
