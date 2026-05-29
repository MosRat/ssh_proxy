use anyhow::{Result, bail};

use crate::cli;

pub(crate) fn parse_remote_os(value: &str) -> Result<cli::RemoteOs> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::RemoteOs::Auto),
        "unix" | "linux" | "macos" => Ok(cli::RemoteOs::Unix),
        "windows" => Ok(cli::RemoteOs::Windows),
        other => bail!("invalid remote_os value {other:?}"),
    }
}

#[cfg(test)]
pub(crate) fn parse_remote_transport(value: &str) -> Result<cli::RemoteTransport> {
    ssh_proxy_route::parse_transport_mode(value)
        .map(Into::into)
        .map_err(anyhow::Error::msg)
}
