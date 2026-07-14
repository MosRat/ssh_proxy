use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

pub fn init(filter: &str) -> Result<()> {
    let env_filter = EnvFilter::try_new(filter)
        .or_else(|_| EnvFilter::try_new("info"))
        .context("failed to create log filter")?;
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .compact()
        .init();
    Ok(())
}
