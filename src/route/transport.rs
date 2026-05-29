use anyhow::{Result, bail};

use crate::{cli, peer_lifecycle};

pub(crate) fn remote_transport_name(transport: cli::RemoteTransport) -> &'static str {
    peer_lifecycle::connection::remote_transport_name(transport)
}

pub(crate) use peer_lifecycle::connection::{
    direct_transport_policy, direct_transport_policy_reason, ssh_data_plane_reason, ssh_mode_name,
    ssh_mode_reason, tls_peer_auth_mode,
};

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
    peer_lifecycle::connection::parse_remote_transport(value)
}
