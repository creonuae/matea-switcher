use anyhow::Result;
use tracing::info;

mod classifier;
mod context;
mod mapper;
mod platform;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    info!("MaTea v{} starting", env!("CARGO_PKG_VERSION"));

    // v0.1 roadmap внутри:
    //   1) open /dev/input/event* via evdev (нужна группа input)
    //   2) accumulate word buffer on key events
    //   3) on word boundary → classifier (Hunspell + n-gram)
    //   4) if `flip` → grab input, write backspace×N + corrected text via uinput
    //
    // Пока — wire up trait Platform и вернуть заглушку.

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
