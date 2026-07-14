use anyhow::Result;
use ssh_proxy_route::RoutePoolSizingInput;

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
    let input = pool_input(args, profile, defaults);
    let policy = ssh_proxy_route::plan_transport_pool(&input);
    TransportPoolPolicy {
        size: policy.size,
        source: policy.source,
        reason: policy.reason,
        pool_policy: policy.pool_policy,
        workload_hint: policy.workload_hint.into(),
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

pub(super) fn pool_policy_name(hint: cli::RouteWorkloadHint) -> &'static str {
    ssh_proxy_route::pool_policy_name(hint.into())
}

pub(super) fn ssh_session_pool_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> SshSessionPoolPolicy {
    let input = pool_input(args, profile, defaults);
    let policy = ssh_proxy_route::plan_ssh_session_pool(&input);
    SshSessionPoolPolicy {
        size: policy.size,
        source: policy.source,
        reason: policy.reason,
        warning: policy.warning,
    }
}

fn pool_input(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> RoutePoolSizingInput {
    RoutePoolSizingInput {
        has_tcp_target: args.tcp_target.is_some(),
        command_transport_pool_size: args.transport_pool_size,
        profile_transport_pool_size: profile.and_then(|profile| profile.transport_pool_size),
        default_transport_pool_size: defaults.transport_pool_size,
        command_ssh_session_pool_size: args.ssh_session_pool_size,
        profile_ssh_session_pool_size: profile.and_then(|profile| profile.ssh_session_pool_size),
        default_ssh_session_pool_size: defaults.ssh_session_pool_size,
        command_workload_hint: args.workload_hint.map(Into::into),
        profile_workload_hint: profile.and_then(|profile| profile.workload_hint),
        default_workload_hint: defaults.workload_hint,
    }
}
