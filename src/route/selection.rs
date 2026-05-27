use std::net::SocketAddr;

use anyhow::{Result, bail};

use crate::{cli, config};

use super::{parse_remote_transport, remote_transport_name};

#[derive(Debug, Clone)]
pub(crate) struct TransportSelection {
    pub(crate) transport: cli::RemoteTransport,
    pub(crate) source: String,
    pub(crate) reason: String,
}

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
) -> Result<TransportSelection> {
    if args.remote_transport != cli::RemoteTransport::Auto {
        return Ok(TransportSelection {
            transport: args.remote_transport,
            source: "cli".to_string(),
            reason: format!(
                "selected by --remote-transport {}",
                remote_transport_name(args.remote_transport)
            ),
        });
    }

    if let Some(value) = profile.and_then(|profile| profile.remote_transport.as_deref()) {
        let transport = parse_remote_transport(value)?;
        if transport != cli::RemoteTransport::Auto {
            return Ok(TransportSelection {
                transport,
                source: "profile".to_string(),
                reason: "selected by target profile remote_transport".to_string(),
            });
        }
    }

    if let Some(addr) = remote_tls {
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::TlsTcp,
            source: "topology".to_string(),
            reason: format!(
                "direct TLS/TCP peer endpoint {addr} is configured; TLS is the production direct default"
            ),
        });
    }

    if let Some(addr) = remote_quic {
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::Quic,
            source: "topology".to_string(),
            reason: format!(
                "direct QUIC peer endpoint {addr} is configured; framed QUIC is selected while quic-native remains opt-in"
            ),
        });
    }

    if let Some(value) = defaults.remote_transport.as_deref() {
        let transport = parse_remote_transport(value)?;
        match transport {
            cli::RemoteTransport::Auto => {}
            cli::RemoteTransport::PlainTcp if allow_plain_tcp => {
                let source = plain_tcp_auto_source(args, profile, defaults)
                    .unwrap_or("benchmark-tuned default");
                return Ok(TransportSelection {
                    transport,
                    source: source.to_string(),
                    reason: plain_tcp_selection_reason(source),
                });
            }
            cli::RemoteTransport::PlainTcp => {}
            _ => {
                return Ok(TransportSelection {
                    transport,
                    source: "defaults".to_string(),
                    reason: "selected by [defaults].remote_transport".to_string(),
                });
            }
        }
    }

    if allow_plain_tcp {
        let source = plain_tcp_auto_source(args, profile, defaults).unwrap_or("cli");
        return Ok(TransportSelection {
            transport: cli::RemoteTransport::PlainTcp,
            source: source.to_string(),
            reason: plain_tcp_selection_reason(source),
        });
    }

    let workload = if args.tcp_target.is_some() {
        "fixed --tcp-target route"
    } else if remote_side_listens {
        "remote-owned proxy route"
    } else {
        "SOCKS/HTTP proxy route"
    };
    Ok(TransportSelection {
        transport: cli::RemoteTransport::SshNative,
        source: "topology".to_string(),
        reason: format!(
            "no reachable direct peer transport is configured for this {workload}; using ssh-native direct-tcpip as the SSH-only simple egress default"
        ),
    })
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

fn plain_tcp_auto_source(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> Option<&'static str> {
    if args.allow_plain_tcp {
        Some("cli")
    } else if profile.and_then(|profile| profile.allow_plain_tcp) == Some(true) {
        Some("profile")
    } else if defaults.allow_plain_tcp == Some(true) {
        Some("benchmark-tuned default")
    } else {
        None
    }
}

fn plain_tcp_selection_reason(source: &str) -> String {
    format!(
        "plain TCP peer transport is enabled by {source}; use only for lab or private trusted links"
    )
}

fn parse_deploy(value: &str) -> Result<cli::DeployMode> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::DeployMode::Auto),
        "always" => Ok(cli::DeployMode::Always),
        "never" => Ok(cli::DeployMode::Never),
        other => bail!("invalid deploy value {other:?}"),
    }
}
