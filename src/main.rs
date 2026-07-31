use anyhow::Result;
use clap::Parser;
use temperature_sensor::{app, cli::Cli};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    app::run(Cli::parse()).await
}
