use std::net::SocketAddr;

use anyhow::{Error, Result, bail};
use ssh_proxy_route::{RemoteUseConnectMode, RemoteUseInput};

use crate::{cli, config, peer_lifecycle};

pub(crate) use peer_lifecycle::connection::TransportSelection;
pub(crate) use ssh_proxy_route::{RemoteUseDecision, RemoteUsePlan};

pub(crate) fn transport_selection_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
    remote_quic: Option<SocketAddr>,
    remote_tls: Option<SocketAddr>,
    allow_plain_tcp: bool,
    remote_side_listens: bool,
    persistent_peer_ready: bool,
) -> Result<TransportSelection> {
    peer_lifecycle::connection::transport_selection_policy(
        args,
        profile,
        defaults,
        remote_quic,
        remote_tls,
        allow_plain_tcp,
        remote_side_listens,
        persistent_peer_ready,
    )
}

pub(crate) fn route_deploy_mode(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<cli::DeployMode> {
    if args.deploy != cli::DeployMode::Auto {
        return Ok(args.deploy);
    }
    config
        .defaults
        .deploy
        .as_deref()
        .map(parse_deploy)
        .transpose()
        .map(|value| value.unwrap_or(cli::DeployMode::Auto))
}

pub(crate) fn remote_use_decision(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<RemoteUseDecision> {
    let input = remote_use_input(args, config);
    let decision = ssh_proxy_route::decide_remote_use(&input).map_err(Error::msg)?;
    if matches!(args.connect_mode, cli::RouteConnectMode::Auto)
        && matches!(decision.plan, RemoteUsePlan::ReverseLink)
    {
        if let Some(err) = decision.fallback_reason.as_deref() {
            tracing::info!(
                error = %err,
                "direct remote-uses-local peer transport is unavailable; using local-initiated reverse link"
            );
        }
    }
    Ok(decision)
}

#[cfg(test)]
pub(crate) fn local_peer_addr(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<SocketAddr> {
    let input = remote_use_input(args, config);
    ssh_proxy_route::resolve_remote_use_local_peer(&input).map_err(Error::msg)
}

fn remote_use_input(args: &cli::RouteArgs, config: &config::AppConfig) -> RemoteUseInput {
    RemoteUseInput {
        connect_mode: remote_use_connect_mode(args.connect_mode),
        local_peer: args.local_peer,
        daemon_transport_listen: config.daemon.transport_listen,
    }
}

fn remote_use_connect_mode(mode: cli::RouteConnectMode) -> RemoteUseConnectMode {
    match mode {
        cli::RouteConnectMode::Auto => RemoteUseConnectMode::Auto,
        cli::RouteConnectMode::Direct => RemoteUseConnectMode::Direct,
        cli::RouteConnectMode::ReverseLink => RemoteUseConnectMode::ReverseLink,
    }
}

fn parse_deploy(value: &str) -> Result<cli::DeployMode> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::DeployMode::Auto),
        "always" => Ok(cli::DeployMode::Always),
        "never" => Ok(cli::DeployMode::Never),
        other => bail!("invalid deploy value {other:?}"),
    }
}
