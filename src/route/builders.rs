use std::net::SocketAddr;

use anyhow::Result;

use crate::{cli, config, peer_transport};

use super::policy::{quic_transport_policy, ssh_session_pool_policy, transport_pool_policy};
use super::selection::{route_deploy_mode, transport_selection_policy};
use super::transport::parse_remote_os;

pub(crate) fn remote_direct_host_args(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
    local_peer: SocketAddr,
    token: String,
) -> Result<cli::HostArgs> {
    let mut forward = node_forward_from_route(args, config, "local-egress".to_string(), true)?;
    forward.remote_tcp = local_peer;
    forward.remote_token = Some(token);
    if forward.remote_quic.is_none() && forward.remote_tls.is_none() {
        forward.allow_plain_tcp = args.allow_plain_tcp;
        forward.remote_transport = if forward.allow_plain_tcp {
            cli::RemoteTransport::PlainTcp
        } else {
            cli::RemoteTransport::Auto
        };
        forward.transport_selection_source = Some(if args.allow_plain_tcp {
            "cli".to_string()
        } else {
            "topology".to_string()
        });
        forward.transport_selection_reason = Some(if forward.allow_plain_tcp {
            format!(
                "remote direct route uses local peer {local_peer}; plain TCP is allowed only because it was explicitly enabled for lab or private trusted links"
            )
        } else {
            format!(
                "remote direct route uses local peer {local_peer}; no direct TLS/QUIC material is configured, so peer transport auto-selection remains unresolved"
            )
        });
    }
    Ok(cli::HostArgs {
        target: args.target.clone(),
        command: cli::HostCommand::NodeForward(forward),
        ssh_args: args.ssh_args.clone(),
        user: args.user.clone(),
        port: args.ssh_port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
        remote_path: args.remote_path.clone(),
        remote_bin: args.remote_bin.clone(),
        remote_os: args.remote_os,
        remote_token: None,
        remote_tcp: args
            .remote_tcp
            .or(config.defaults.remote_tcp)
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 19080))),
        remote_control: args
            .remote_control
            .or(config.defaults.remote_control)
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 19081))),
        remote_tls_transport: None,
        remote_quic_transport: None,
        remote_tls_cert: None,
        remote_tls_key: None,
        remote_tls_client_ca: None,
        persist: cli::PersistMode::Auto,
    })
}

pub(crate) fn install_args_from_route(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<cli::InstallRemoteArgs> {
    let defaults = &config.defaults;
    let profile = config.profiles.get(&args.target);
    let mut install = cli::InstallRemoteArgs {
        target: args.target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: None,
        user: args.user.clone(),
        port: args.ssh_port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new || defaults.accept_new.unwrap_or(false),
        insecure_ignore_host_key: args.insecure_ignore_host_key
            || defaults.insecure_ignore_host_key.unwrap_or(false),
        jump: args.jump.clone(),
        remote_path: args
            .remote_path
            .clone()
            .or_else(|| profile.and_then(|profile| profile.remote_path.clone()))
            .or_else(|| defaults.remote_path.clone()),
        remote_bin: args
            .remote_bin
            .clone()
            .or_else(|| {
                profile.and_then(|profile| profile.remote_bin.as_ref().map(config::expand_path))
            })
            .or_else(|| defaults.remote_bin.as_ref().map(config::expand_path)),
        remote_os: if args.remote_os == cli::RemoteOs::Auto {
            profile
                .and_then(|profile| profile.remote_os.as_deref())
                .or(defaults.remote_os.as_deref())
                .map(parse_remote_os)
                .transpose()?
                .unwrap_or(cli::RemoteOs::Auto)
        } else {
            args.remote_os
        },
        remote_token: args
            .remote_token
            .clone()
            .or_else(|| profile.and_then(|profile| profile.remote_token.clone()))
            .or_else(|| defaults.remote_token.clone()),
        remote_tcp: args
            .remote_tcp
            .or_else(|| profile.and_then(|profile| profile.remote_tcp))
            .or(defaults.remote_tcp)
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 19080))),
        remote_control: args
            .remote_control
            .or_else(|| profile.and_then(|profile| profile.remote_control))
            .or(defaults.remote_control)
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 19081))),
        local_node_id: None,
        local_node_name: None,
        local_control_endpoint: None,
        local_transport: None,
        remote_node_id: None,
        remote_node_name: None,
        remote_tls_transport: None,
        remote_quic_transport: None,
        remote_tls_cert: None,
        remote_tls_key: None,
        remote_tls_client_ca: None,
        persist: cli::PersistMode::Auto,
    };
    config.apply_install_defaults(&mut install, Some(&args.target))?;
    Ok(install)
}

pub(crate) fn node_forward_from_route(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
    target: String,
    remote_side_listens: bool,
) -> Result<cli::NodeForwardArgs> {
    let listen = SocketAddr::new(args.bind, args.port);
    let defaults = &config.defaults;
    let profile = config.profiles.get(&args.target);
    let runtime = route_runtime(args, config)?;
    let remote_tcp = args
        .remote_tcp
        .or_else(|| profile.and_then(|profile| profile.remote_tcp))
        .or(defaults.remote_tcp)
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 19080)));
    let remote_quic = args
        .remote_quic
        .or_else(|| profile.and_then(|profile| profile.remote_quic))
        .or(defaults.remote_quic);
    let allow_plain_tcp = args.allow_plain_tcp
        || profile
            .and_then(|profile| profile.allow_plain_tcp)
            .unwrap_or(false)
        || defaults.allow_plain_tcp.unwrap_or(false);
    let remote_tls = args
        .remote_tls
        .or_else(|| profile.and_then(|profile| profile.remote_tls))
        .or(defaults.remote_tls);
    let transport_selection = transport_selection_policy(
        args,
        profile,
        defaults,
        remote_quic,
        remote_tls,
        allow_plain_tcp,
        remote_side_listens,
        config.peers.get(&args.target).is_some_and(|peer| {
            peer.remote_path.is_some()
                && peer.control_endpoint.is_some()
                && (peer.transport.is_some()
                    || peer.tls_transport.is_some()
                    || peer.quic_transport.is_some())
        }),
    )?;
    Ok(cli::NodeForwardArgs {
        target,
        listen,
        tcp_target: args.tcp_target.clone(),
        ssh_args: args.ssh_args.clone(),
        user: args.user.clone(),
        port: args.ssh_port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new || defaults.accept_new.unwrap_or(false),
        insecure_ignore_host_key: args.insecure_ignore_host_key
            || defaults.insecure_ignore_host_key.unwrap_or(false),
        jump: args.jump.clone(),
        remote_path: args
            .remote_path
            .clone()
            .or_else(|| profile.and_then(|profile| profile.remote_path.clone()))
            .or_else(|| defaults.remote_path.clone()),
        remote_bin: args
            .remote_bin
            .clone()
            .or_else(|| {
                profile.and_then(|profile| profile.remote_bin.as_ref().map(config::expand_path))
            })
            .or_else(|| defaults.remote_bin.as_ref().map(config::expand_path)),
        deploy: route_deploy_mode(args, config)?,
        remote_os: if args.remote_os == cli::RemoteOs::Auto {
            profile
                .and_then(|profile| profile.remote_os.as_deref())
                .or(defaults.remote_os.as_deref())
                .map(parse_remote_os)
                .transpose()?
                .unwrap_or(cli::RemoteOs::Auto)
        } else {
            args.remote_os
        },
        remote_transport: transport_selection.transport,
        remote_tcp,
        remote_control: args
            .remote_control
            .or_else(|| profile.and_then(|profile| profile.remote_control))
            .or(defaults.remote_control)
            .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 19081))),
        remote_quic,
        allow_plain_tcp,
        remote_tls,
        remote_ca: args
            .remote_ca
            .clone()
            .or_else(|| {
                profile.and_then(|profile| profile.remote_ca.as_ref().map(config::expand_path))
            })
            .or_else(|| defaults.remote_ca.as_ref().map(config::expand_path)),
        remote_name: if args.remote_name == "localhost" {
            profile
                .and_then(|profile| profile.remote_name.clone())
                .or_else(|| defaults.remote_name.clone())
                .unwrap_or_else(|| "localhost".to_string())
        } else {
            args.remote_name.clone()
        },
        remote_client_cert: profile
            .and_then(|profile| profile.remote_client_cert.as_ref().map(config::expand_path))
            .or_else(|| {
                defaults
                    .remote_client_cert
                    .as_ref()
                    .map(config::expand_path)
            }),
        remote_client_key: profile
            .and_then(|profile| profile.remote_client_key.as_ref().map(config::expand_path))
            .or_else(|| defaults.remote_client_key.as_ref().map(config::expand_path)),
        remote_token: args
            .remote_token
            .clone()
            .or_else(|| profile.and_then(|profile| profile.remote_token.clone()))
            .or_else(|| defaults.remote_token.clone()),
        egress_proxy: args
            .egress_proxy
            .clone()
            .or_else(|| profile.and_then(|profile| profile.egress_proxy.clone()))
            .or_else(|| defaults.egress_proxy.clone()),
        reconnect_delay_secs: runtime.reconnect_delay_secs,
        reconnect_max_delay_secs: runtime.reconnect_max_delay_secs,
        connect_timeout_secs: runtime.connect_timeout_secs,
        transport_pool_size: runtime.transport_pool_size,
        quic_max_bidi_streams: runtime.quic_options.max_bidi_streams,
        quic_stream_receive_window: runtime.quic_options.stream_receive_window,
        quic_receive_window: runtime.quic_options.receive_window,
        quic_keep_alive_interval_secs: runtime.quic_options.keep_alive_interval_secs,
        quic_idle_timeout_secs: runtime.quic_options.idle_timeout_secs,
        ssh_session_pool_size: Some(runtime.ssh_session_pool_size),
        ssh_session_pool_source: Some(runtime.ssh_session_pool_source.clone()),
        ssh_session_pool_reason: Some(runtime.ssh_session_pool_reason.clone()),
        ssh_session_pool_warning: runtime.ssh_session_pool_warning.clone(),
        transport_pool_source: Some(runtime.transport_pool_source.clone()),
        transport_pool_reason: Some(runtime.transport_pool_reason.clone()),
        pool_policy: Some(runtime.pool_policy.clone()),
        workload_hint: Some(runtime.workload_hint),
        transport_selection_source: Some(transport_selection.source),
        transport_selection_reason: Some(transport_selection.reason),
        preflight_recommended_fallback: None,
        preflight_selected_reason: None,
        preflight_repair_hint: None,
        preflight_candidate_failures: Vec::new(),
        no_reconnect: runtime.no_reconnect,
        id: Some(route_id(
            args,
            if remote_side_listens {
                "remote-via-local"
            } else {
                "local-via-remote"
            },
        )),
        volatile: args.volatile,
    })
}

pub(crate) fn node_reverse_from_route(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<cli::NodeReverseArgs> {
    let defaults = &config.defaults;
    let profile = config.profiles.get(&args.target);
    let runtime = route_runtime(args, config)?;
    Ok(cli::NodeReverseArgs {
        target: args.target.clone(),
        remote_listen: SocketAddr::new(args.bind, args.port),
        tcp_target: args.tcp_target.clone(),
        ssh_args: args.ssh_args.clone(),
        user: args.user.clone(),
        port: args.ssh_port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new || defaults.accept_new.unwrap_or(false),
        insecure_ignore_host_key: args.insecure_ignore_host_key
            || defaults.insecure_ignore_host_key.unwrap_or(false),
        jump: args.jump.clone(),
        remote_path: args
            .remote_path
            .clone()
            .or_else(|| profile.and_then(|profile| profile.remote_path.clone()))
            .or_else(|| defaults.remote_path.clone()),
        remote_bin: args
            .remote_bin
            .clone()
            .or_else(|| {
                profile.and_then(|profile| profile.remote_bin.as_ref().map(config::expand_path))
            })
            .or_else(|| defaults.remote_bin.as_ref().map(config::expand_path)),
        deploy: route_deploy_mode(args, config)?,
        remote_os: if args.remote_os == cli::RemoteOs::Auto {
            profile
                .and_then(|profile| profile.remote_os.as_deref())
                .or(defaults.remote_os.as_deref())
                .map(parse_remote_os)
                .transpose()?
                .unwrap_or(cli::RemoteOs::Auto)
        } else {
            args.remote_os
        },
        egress_proxy: args
            .egress_proxy
            .clone()
            .or_else(|| profile.and_then(|profile| profile.egress_proxy.clone()))
            .or_else(|| defaults.egress_proxy.clone()),
        reconnect_delay_secs: runtime.reconnect_delay_secs,
        reconnect_max_delay_secs: runtime.reconnect_max_delay_secs,
        connect_timeout_secs: runtime.connect_timeout_secs,
        transport_pool_source: Some("fixed".to_string()),
        transport_pool_reason: Some(
            "reverse-link currently uses one SSH-established route link; transport pooling applies to forward peer transports"
                .to_string(),
        ),
        no_reconnect: runtime.no_reconnect,
        id: Some(route_id(args, "remote-via-local-reverse-link")),
        volatile: args.volatile,
    })
}

#[derive(Debug, Clone)]
struct RouteRuntime {
    reconnect_delay_secs: u64,
    reconnect_max_delay_secs: u64,
    connect_timeout_secs: u64,
    transport_pool_size: usize,
    quic_options: peer_transport::QuicTransportOptions,
    ssh_session_pool_size: usize,
    ssh_session_pool_source: String,
    ssh_session_pool_reason: String,
    ssh_session_pool_warning: Option<String>,
    transport_pool_source: String,
    transport_pool_reason: String,
    pool_policy: String,
    workload_hint: cli::RouteWorkloadHint,
    no_reconnect: bool,
}

fn route_runtime(args: &cli::RouteArgs, config: &config::AppConfig) -> Result<RouteRuntime> {
    let defaults = &config.defaults;
    let profile = config.profiles.get(&args.target);
    let reconnect_delay_secs = args
        .reconnect_delay_secs
        .or_else(|| profile.and_then(|profile| profile.reconnect_delay_secs))
        .or(defaults.reconnect_delay_secs)
        .unwrap_or(5);
    let reconnect_max_delay_secs = args
        .reconnect_max_delay_secs
        .or_else(|| profile.and_then(|profile| profile.reconnect_max_delay_secs))
        .or(defaults.reconnect_max_delay_secs)
        .unwrap_or(60)
        .max(reconnect_delay_secs);
    let connect_timeout_secs = args
        .connect_timeout_secs
        .or_else(|| profile.and_then(|profile| profile.connect_timeout_secs))
        .or(defaults.connect_timeout_secs)
        .unwrap_or(30);
    let transport_pool = transport_pool_policy(args, profile, defaults);
    let quic_options = quic_transport_policy(args, profile, defaults)?;
    let ssh_session_pool = ssh_session_pool_policy(args, profile, defaults);
    let no_reconnect = args.no_reconnect
        || profile
            .and_then(|profile| profile.no_reconnect)
            .or(defaults.no_reconnect)
            .unwrap_or(false);

    Ok(RouteRuntime {
        reconnect_delay_secs,
        reconnect_max_delay_secs,
        connect_timeout_secs,
        transport_pool_size: transport_pool.size,
        quic_options,
        ssh_session_pool_size: ssh_session_pool.size,
        ssh_session_pool_source: ssh_session_pool.source,
        ssh_session_pool_reason: ssh_session_pool.reason,
        ssh_session_pool_warning: ssh_session_pool.warning,
        transport_pool_source: transport_pool.source,
        transport_pool_reason: transport_pool.reason,
        pool_policy: transport_pool.pool_policy,
        workload_hint: transport_pool.workload_hint,
        no_reconnect,
    })
}

pub(crate) fn route_id(args: &cli::RouteArgs, prefix: &str) -> String {
    args.id
        .clone()
        .unwrap_or_else(|| format!("{prefix}:{}:{}", args.target, args.port))
}
