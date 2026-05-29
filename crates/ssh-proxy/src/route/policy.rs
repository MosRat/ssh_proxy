use anyhow::Result;

use crate::{cli, config, peer_transport};

#[derive(Debug, Clone)]
pub(super) struct TransportPoolPolicy {
    pub(super) size: usize,
    pub(super) source: String,
    pub(super) reason: String,
    pub(super) pool_policy: String,
    pub(super) workload_hint: cli::RouteWorkloadHint,
}

#[derive(Debug, Clone)]
pub(super) struct SshSessionPoolPolicy {
    pub(super) size: usize,
    pub(super) source: String,
    pub(super) reason: String,
    pub(super) warning: Option<String>,
}

pub(super) fn transport_pool_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> TransportPoolPolicy {
    if let Some(value) = args.transport_pool_size {
        let size = value.max(1);
        return TransportPoolPolicy {
            size,
            source: "command-line".to_string(),
            reason: pool_reason("--transport-pool-size", value, size),
            pool_policy: "explicit".to_string(),
            workload_hint: workload_hint_policy(args, profile, defaults),
        };
    }
    if let Some(value) = profile.and_then(|profile| profile.transport_pool_size) {
        let size = value.max(1);
        return TransportPoolPolicy {
            size,
            source: "profile".to_string(),
            reason: pool_reason("target profile transport_pool_size", value, size),
            pool_policy: "explicit".to_string(),
            workload_hint: workload_hint_policy(args, profile, defaults),
        };
    }
    if let Some(value) = defaults.transport_pool_size {
        let size = value.max(1);
        return TransportPoolPolicy {
            size,
            source: "defaults".to_string(),
            reason: pool_reason("[defaults].transport_pool_size", value, size),
            pool_policy: "explicit".to_string(),
            workload_hint: workload_hint_policy(args, profile, defaults),
        };
    }
    let hint = workload_hint_policy(args, profile, defaults);
    TransportPoolPolicy {
        size: implicit_transport_pool_size(args, hint),
        source: "implicit".to_string(),
        reason: implicit_transport_pool_reason(args, hint),
        pool_policy: pool_policy_name(hint).to_string(),
        workload_hint: hint,
    }
}

pub(super) fn quic_transport_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> Result<peer_transport::QuicTransportOptions> {
    peer_transport::QuicTransportOptions::new(
        args.quic_max_bidi_streams
            .or_else(|| profile.and_then(|profile| profile.quic_max_bidi_streams))
            .or(defaults.quic_max_bidi_streams)
            .unwrap_or(peer_transport::QUIC_MAX_BIDI_STREAMS),
        args.quic_stream_receive_window
            .or_else(|| profile.and_then(|profile| profile.quic_stream_receive_window))
            .or(defaults.quic_stream_receive_window)
            .unwrap_or(peer_transport::QUIC_STREAM_RECEIVE_WINDOW),
        args.quic_receive_window
            .or_else(|| profile.and_then(|profile| profile.quic_receive_window))
            .or(defaults.quic_receive_window)
            .unwrap_or(peer_transport::QUIC_RECEIVE_WINDOW),
        args.quic_keep_alive_interval_secs
            .or_else(|| profile.and_then(|profile| profile.quic_keep_alive_interval_secs))
            .or(defaults.quic_keep_alive_interval_secs)
            .unwrap_or(peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS),
        args.quic_idle_timeout_secs
            .or_else(|| profile.and_then(|profile| profile.quic_idle_timeout_secs))
            .or(defaults.quic_idle_timeout_secs)
            .unwrap_or(peer_transport::QUIC_IDLE_TIMEOUT_SECS),
    )
}

pub(super) fn workload_hint_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> cli::RouteWorkloadHint {
    args.workload_hint
        .or_else(|| profile.and_then(|profile| profile.workload_hint.map(Into::into)))
        .or(defaults.workload_hint.map(Into::into))
        .unwrap_or_else(|| {
            if args.tcp_target.is_some() {
                cli::RouteWorkloadHint::Large
            } else {
                cli::RouteWorkloadHint::Concurrent
            }
        })
}

fn implicit_transport_pool_size(args: &cli::RouteArgs, hint: cli::RouteWorkloadHint) -> usize {
    match hint {
        cli::RouteWorkloadHint::Large => 1,
        cli::RouteWorkloadHint::Concurrent | cli::RouteWorkloadHint::Mixed => {
            if args.tcp_target.is_some() { 1 } else { 4 }
        }
    }
}

fn implicit_transport_pool_reason(args: &cli::RouteArgs, hint: cli::RouteWorkloadHint) -> String {
    match (args.tcp_target.is_some(), hint) {
        (true, cli::RouteWorkloadHint::Large) => {
            "pool_policy=large: implicit single-worker default for fixed --tcp-target routes"
                .to_string()
        }
        (true, _) => {
            format!(
                "pool_policy={}: fixed --tcp-target routes stay at pool=1 unless --transport-pool-size is explicit",
                pool_policy_name(hint)
            )
        }
        (false, cli::RouteWorkloadHint::Large) => {
            "pool_policy=large: single-worker default favors one large transfer".to_string()
        }
        (false, cli::RouteWorkloadHint::Concurrent) => {
            "pool_policy=concurrent: implicit pool=4 default for multi-flow SOCKS/HTTP proxy routes"
                .to_string()
        }
        (false, cli::RouteWorkloadHint::Mixed) => {
            "pool_policy=mixed: implicit pool=4 default balances large and concurrent proxy traffic"
                .to_string()
        }
    }
}

pub(super) fn pool_policy_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}

pub(super) fn ssh_session_pool_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> SshSessionPoolPolicy {
    if let Some(value) = args.ssh_session_pool_size {
        let size = value.max(1);
        return SshSessionPoolPolicy {
            size,
            source: "command-line".to_string(),
            reason: pool_reason("--ssh-session-pool-size", value, size),
            warning: ssh_session_pool_warning(size),
        };
    }
    if let Some(value) = profile.and_then(|profile| profile.ssh_session_pool_size) {
        let size = value.max(1);
        return SshSessionPoolPolicy {
            size,
            source: "profile".to_string(),
            reason: pool_reason("target profile ssh_session_pool_size", value, size),
            warning: ssh_session_pool_warning(size),
        };
    }
    if let Some(value) = defaults.ssh_session_pool_size {
        let requested = value.max(1);
        let size = requested.min(2);
        return SshSessionPoolPolicy {
            size,
            source: "defaults".to_string(),
            reason: if requested == size {
                pool_reason("[defaults].ssh_session_pool_size", value, size)
            } else {
                format!(
                    "loaded from [defaults].ssh_session_pool_size={value}; capped to pool=2 because only command-line/profile benchmark experiments may exceed the implicit-safe ssh-native range"
                )
            },
            warning: if requested > size {
                Some(
                    "ssh-native defaults above 2 are not auto-selected; use --ssh-session-pool-size or a target profile for explicit benchmark experiments"
                        .to_string(),
                )
            } else {
                ssh_session_pool_warning(size)
            },
        };
    }

    let size = implicit_ssh_session_pool_size(args);
    SshSessionPoolPolicy {
        size,
        source: "implicit".to_string(),
        reason: implicit_ssh_session_pool_reason(args),
        warning: None,
    }
}

fn implicit_ssh_session_pool_size(args: &cli::RouteArgs) -> usize {
    if args.tcp_target.is_some() { 1 } else { 2 }
}

fn implicit_ssh_session_pool_reason(args: &cli::RouteArgs) -> String {
    if args.tcp_target.is_some() {
        "implicit ssh-native single-session default for fixed --tcp-target routes".to_string()
    } else {
        "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes".to_string()
    }
}

fn ssh_session_pool_warning(size: usize) -> Option<String> {
    (size > 2).then(|| {
        "ssh-native session pools above 2 can lose to handshake and scheduling overhead; benchmark before relying on this explicit value"
            .to_string()
    })
}

fn pool_reason(source: &str, requested: usize, effective: usize) -> String {
    if requested == effective {
        format!("loaded from {source}")
    } else {
        format!("loaded from {source}; clamped to minimum 1")
    }
}
