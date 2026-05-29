use std::net::SocketAddr;

use anyhow::{Result, bail};

use crate::{cli, config, peer_lifecycle};

pub(crate) use peer_lifecycle::connection::TransportSelection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteUsePlan {
    Direct(SocketAddr),
    ReverseLink,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteUseDecision {
    pub(crate) plan: RemoteUsePlan,
    pub(crate) fallback_reason: Option<String>,
}

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
    match args.connect_mode {
        cli::RouteConnectMode::ReverseLink => {
            return Ok(RemoteUseDecision {
                plan: RemoteUsePlan::ReverseLink,
                fallback_reason: Some("--connect-mode reverse-link requested".to_string()),
            });
        }
        cli::RouteConnectMode::Direct => {
            return local_peer_addr(args, config).map(|addr| RemoteUseDecision {
                plan: RemoteUsePlan::Direct(addr),
                fallback_reason: None,
            });
        }
        cli::RouteConnectMode::Auto => {}
    }

    match local_peer_addr(args, config) {
        Ok(addr) => Ok(RemoteUseDecision {
            plan: RemoteUsePlan::Direct(addr),
            fallback_reason: None,
        }),
        Err(err) => {
            tracing::info!(
                error = %err,
                "direct remote-uses-local peer transport is unavailable; using local-initiated reverse link"
            );
            Ok(RemoteUseDecision {
                plan: RemoteUsePlan::ReverseLink,
                fallback_reason: Some(err.to_string()),
            })
        }
    }
}

pub(crate) fn local_peer_addr(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<SocketAddr> {
    if let Some(addr) = args.local_peer {
        return Ok(addr);
    }
    let Some(addr) = config.daemon.transport_listen else {
        bail!(
            "--direction remote-uses-local needs --local-peer or [daemon].transport_listen; run `ssh_proxy daemon install --scope system --elevate` first"
        );
    };
    if addr.ip().is_loopback() {
        bail!(
            "local daemon transport {addr} is loopback-only; pass --local-peer <reachable-ip:port>, or use a public/TLS/QUIC relay route when this machine is behind NAT"
        );
    }
    Ok(addr)
}

fn parse_deploy(value: &str) -> Result<cli::DeployMode> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::DeployMode::Auto),
        "always" => Ok(cli::DeployMode::Always),
        "never" => Ok(cli::DeployMode::Never),
        other => bail!("invalid deploy value {other:?}"),
    }
}
