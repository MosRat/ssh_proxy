use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{net::TcpStream, time};

use crate::{cli, config, control_socket, node_daemon, peer_transport, quic_stream, ssh_client};

pub async fn run(args: cli::RouteArgs, config: config::AppConfig) -> Result<()> {
    let endpoint = control_socket::ControlEndpoint::parse(&args.endpoint)?;
    if args.explain {
        let plan = explain_plan(&args, &config).await?;
        if args.json {
            println!("{}", serde_json::to_string(&plan)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(());
    }
    let mut request = route_intent_request(args.clone());
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&request)?);
        return Ok(());
    }
    if endpoint.is_tcp() {
        node_daemon::attach_auth_token(
            &mut request,
            args.token.as_deref().or(config.daemon.token.as_deref()),
        );
    }
    let response = control_socket::request(&endpoint, &format!("{request}\n"))
        .await
        .with_context(|| {
            format!(
                "failed to contact local daemon at {}; run `ssh_proxy service install` first",
                args.endpoint
            )
        })?;
    print!("{response}");
    Ok(())
}

pub(crate) async fn explain_plan(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<Value> {
    match args.direction {
        cli::RouteDirection::LocalUsesRemote => {
            let mut forward = node_forward_from_route(args, config, args.target.clone(), false)?;
            let id = route_id(args, "local-via-remote");
            let mut plan = local_uses_remote_plan(args, &id, &forward);
            add_local_transport_probe_results(&mut plan, &mut forward).await;
            apply_local_forward_fallback(&mut forward, &mut plan);
            Ok(plan)
        }
        cli::RouteDirection::RemoteUsesLocal => {
            let decision = remote_use_decision(args, config)?;
            match decision.plan {
                RemoteUsePlan::ReverseLink => {
                    let reverse = node_reverse_from_route(args, config)?;
                    let id = route_id(args, "remote-via-local-reverse-link");
                    Ok(remote_uses_local_reverse_link_plan(
                        args,
                        &id,
                        &reverse,
                        decision.fallback_reason.as_deref(),
                    ))
                }
                RemoteUsePlan::Direct(local_peer) => {
                    let token = config
                        .daemon
                        .token
                        .clone()
                        .unwrap_or_else(|| "<daemon-token>".to_string());
                    let host_args = remote_direct_host_args(args, config, local_peer, token)?;
                    match &host_args.command {
                        cli::HostCommand::NodeForward(forward) => {
                            Ok(remote_uses_local_direct_plan(
                                args,
                                forward.id.as_deref().unwrap_or("remote-direct"),
                                forward,
                                local_peer,
                            ))
                        }
                        _ => bail!("unexpected remote direct route command"),
                    }
                }
            }
        }
    }
}

pub(crate) fn route_intent_request(args: cli::RouteArgs) -> serde_json::Value {
    node_daemon::NodeRequest::route_intent(args)
        .to_value()
        .expect("route intent request should serialize")
}

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

#[derive(Debug, Clone)]
struct TransportPoolPolicy {
    size: usize,
    source: String,
    reason: String,
    pool_policy: String,
    workload_hint: cli::RouteWorkloadHint,
}

fn transport_pool_policy(
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

fn quic_transport_policy(
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

fn workload_hint_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
) -> cli::RouteWorkloadHint {
    args.workload_hint
        .or_else(|| profile.and_then(|profile| profile.workload_hint))
        .or(defaults.workload_hint)
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

fn pool_policy_name(hint: cli::RouteWorkloadHint) -> &'static str {
    match hint {
        cli::RouteWorkloadHint::Large => "large",
        cli::RouteWorkloadHint::Concurrent => "concurrent",
        cli::RouteWorkloadHint::Mixed => "mixed",
    }
}

#[derive(Debug, Clone)]
struct SshSessionPoolPolicy {
    size: usize,
    source: String,
    reason: String,
    warning: Option<String>,
}

fn ssh_session_pool_policy(
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

#[derive(Debug, Clone)]
struct TransportSelection {
    transport: cli::RemoteTransport,
    source: String,
    reason: String,
}

fn transport_selection_policy(
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

fn pool_reason(source: &str, requested: usize, effective: usize) -> String {
    if requested == effective {
        format!("loaded from {source}")
    } else {
        format!("loaded from {source}; clamped to minimum 1")
    }
}

fn route_deploy_mode(args: &cli::RouteArgs, config: &config::AppConfig) -> Result<cli::DeployMode> {
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

pub(crate) fn route_start_request(
    id: &str,
    forward: cli::NodeForwardArgs,
    persist: bool,
) -> serde_json::Value {
    let proxy = node_daemon::proxy_args_from_node_forward(forward);
    node_daemon::NodeRequest::route_start_forward(id, persist, proxy)
        .to_value()
        .expect("forward route request should serialize")
}

pub(crate) fn route_start_request_with_reason(
    id: &str,
    forward: cli::NodeForwardArgs,
    persist: bool,
    fallback_reason: Option<String>,
) -> serde_json::Value {
    let proxy = node_daemon::proxy_args_from_node_forward(forward);
    let mut request = node_daemon::NodeRequest::route_start_forward(id, persist, proxy)
        .to_value()
        .expect("forward route request should serialize");
    if let Some(reason) = fallback_reason {
        if let Some(object) = request.as_object_mut() {
            object.insert("fallback_reason".to_string(), json!(reason));
        }
    }
    request
}

pub(crate) fn reverse_route_start_request(
    id: &str,
    reverse: cli::NodeReverseArgs,
    persist: bool,
) -> serde_json::Value {
    let reverse = node_daemon::reverse_args_from_node_reverse(reverse);
    node_daemon::NodeRequest::route_start_reverse(
        id,
        persist,
        reverse,
        Some("reverse-link".to_string()),
    )
    .to_value()
    .expect("reverse route request should serialize")
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

pub(crate) fn local_uses_remote_plan(
    args: &cli::RouteArgs,
    id: &str,
    forward: &cli::NodeForwardArgs,
) -> Value {
    let mut plan = json!({
        "route_id": id,
        "direction": "local-uses-remote",
        "owner": "local",
        "mode": "local-forward",
        "listener": {
            "owner": "local",
            "listen": forward.listen.to_string(),
            "tcp_target": forward.tcp_target.as_ref().map(ToString::to_string),
        },
        "egress": {
            "peer": args.target,
            "side": "remote",
            "upstream_proxy": forward.egress_proxy.clone(),
        },
        "transport_candidates": transport_candidates(forward),
        "selected_transport": remote_transport_name(forward.remote_transport),
        "transport_selection_source": forward
            .transport_selection_source
            .as_deref()
            .unwrap_or("unknown"),
        "transport_selection_reason": forward
            .transport_selection_reason
            .as_deref()
            .unwrap_or("unknown"),
        "direct_transport_policy": direct_transport_policy(forward.remote_transport),
        "direct_transport_policy_reason": direct_transport_policy_reason(forward.remote_transport),
        "tls_peer_auth_mode": tls_peer_auth_mode(
            forward.remote_transport,
            forward.remote_client_cert.as_ref(),
            forward.remote_client_key.as_ref(),
        ),
        "ssh_mode": ssh_mode_name(forward.remote_transport),
        "ssh_mode_reason": ssh_mode_reason(forward.remote_transport),
        "ssh_data_plane_reason": ssh_data_plane_reason(
            forward.remote_transport,
            forward.transport_selection_source.as_deref(),
        ),
        "ssh_session_pool_size": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_size.unwrap_or(1))
        } else {
            Value::Null
        },
        "ssh_session_pool_source": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_source.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_reason": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_reason.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_warning": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_warning.as_deref())
        } else {
            Value::Null
        },
        "topology": topology_hint(args, forward),
        "runtime": route_runtime_plan(
            forward.reconnect_delay_secs,
            forward.reconnect_max_delay_secs,
            forward.connect_timeout_secs,
            forward.transport_pool_size,
            forward.transport_pool_source.as_deref(),
            forward.transport_pool_reason.as_deref(),
            forward.pool_policy.as_deref(),
            forward.workload_hint.map(pool_policy_name),
            forward.no_reconnect,
        ),
        "fallback_reason": Value::Null,
        "next_action": "none",
        "persist": !args.volatile,
    });
    refresh_decision_chain(&mut plan);
    plan
}

pub(crate) fn remote_uses_local_reverse_link_plan(
    args: &cli::RouteArgs,
    id: &str,
    reverse: &cli::NodeReverseArgs,
    fallback_reason: Option<&str>,
) -> Value {
    let mut plan = json!({
        "route_id": id,
        "direction": "remote-uses-local",
        "owner": "local",
        "mode": "reverse-link",
        "listener": {
            "owner": "remote",
            "listen": reverse.remote_listen.to_string(),
            "tcp_target": reverse.tcp_target.as_ref().map(ToString::to_string),
        },
        "egress": {
            "peer": "local",
            "side": "local",
            "upstream_proxy": reverse.egress_proxy.clone(),
        },
        "transport_candidates": ["ssh-reverse-link"],
        "selected_transport": "ssh-reverse-link",
        "runtime": route_runtime_plan(
            reverse.reconnect_delay_secs,
            reverse.reconnect_max_delay_secs,
            reverse.connect_timeout_secs,
            1,
            reverse.transport_pool_source.as_deref(),
            reverse.transport_pool_reason.as_deref(),
            Some("large"),
            Some("large"),
            reverse.no_reconnect,
        ),
        "fallback_reason": fallback_reason,
        "next_action": if fallback_reason.is_some() {
            "set --local-peer <reachable-ip:port> for direct mode"
        } else {
            "none"
        },
        "persist": !args.volatile,
    });
    refresh_decision_chain(&mut plan);
    plan
}

pub(crate) fn remote_uses_local_direct_plan(
    args: &cli::RouteArgs,
    id: &str,
    forward: &cli::NodeForwardArgs,
    local_peer: SocketAddr,
) -> Value {
    let mut plan = json!({
        "route_id": id,
        "direction": "remote-uses-local",
        "owner": "remote",
        "mode": "direct",
        "listener": {
            "owner": "remote",
            "listen": forward.listen.to_string(),
            "tcp_target": forward.tcp_target.as_ref().map(ToString::to_string),
        },
        "egress": {
            "peer": "local",
            "side": "local",
            "reachable_peer": local_peer.to_string(),
            "upstream_proxy": forward.egress_proxy.clone(),
        },
        "transport_candidates": transport_candidates(forward),
        "selected_transport": remote_transport_name(forward.remote_transport),
        "transport_selection_source": forward
            .transport_selection_source
            .as_deref()
            .unwrap_or("unknown"),
        "transport_selection_reason": forward
            .transport_selection_reason
            .as_deref()
            .unwrap_or("unknown"),
        "direct_transport_policy": direct_transport_policy(forward.remote_transport),
        "direct_transport_policy_reason": direct_transport_policy_reason(forward.remote_transport),
        "tls_peer_auth_mode": tls_peer_auth_mode(
            forward.remote_transport,
            forward.remote_client_cert.as_ref(),
            forward.remote_client_key.as_ref(),
        ),
        "ssh_mode": ssh_mode_name(forward.remote_transport),
        "ssh_mode_reason": ssh_mode_reason(forward.remote_transport),
        "ssh_data_plane_reason": ssh_data_plane_reason(
            forward.remote_transport,
            forward.transport_selection_source.as_deref(),
        ),
        "ssh_session_pool_size": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_size.unwrap_or(1))
        } else {
            Value::Null
        },
        "ssh_session_pool_source": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_source.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_reason": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_reason.as_deref().unwrap_or("unknown"))
        } else {
            Value::Null
        },
        "ssh_session_pool_warning": if matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
            json!(forward.ssh_session_pool_warning.as_deref())
        } else {
            Value::Null
        },
        "topology": topology_hint(args, forward),
        "runtime": route_runtime_plan(
            forward.reconnect_delay_secs,
            forward.reconnect_max_delay_secs,
            forward.connect_timeout_secs,
            forward.transport_pool_size,
            forward.transport_pool_source.as_deref(),
            forward.transport_pool_reason.as_deref(),
            forward.pool_policy.as_deref(),
            forward.workload_hint.map(pool_policy_name),
            forward.no_reconnect,
        ),
        "fallback_reason": Value::Null,
        "next_action": "none",
        "persist": !args.volatile,
    });
    refresh_decision_chain(&mut plan);
    plan
}

pub(crate) async fn add_local_transport_probe_results(
    plan: &mut Value,
    forward: &mut cli::NodeForwardArgs,
) {
    let timeout = Duration::from_millis(750);
    let mut results = Vec::new();

    if let Some(addr) = forward.remote_quic {
        results.push(probe_quic_endpoint(forward, addr, timeout).await);
    }

    if let Some(addr) = forward.remote_tls {
        results.push(probe_tcp_endpoint("tls-tcp", addr, timeout).await);
    }

    if forward.allow_plain_tcp {
        results.push(probe_tcp_endpoint("plain-tcp", forward.remote_tcp, timeout).await);
    }

    results.push(json!({
        "protocol": "ssh-direct-tcpip",
        "endpoint": forward.remote_tcp.to_string(),
        "reachable": Value::Null,
        "status": "not-probed",
        "message": "SSH direct-tcpip reachability follows the SSH session and may work even when direct private endpoints do not",
    }));
    results.push(json!({
        "protocol": "ssh-exec",
        "endpoint": Value::Null,
        "reachable": Value::Null,
        "status": "not-probed",
        "message": "SSH exec fallback is validated when the route connects over SSH",
    }));

    let candidate_failures = candidate_failures(&results);
    let direct_failures = candidate_failures.len();
    let direct_successes = results
        .iter()
        .filter(|result| {
            is_direct_probe_protocol(result["protocol"].as_str()) && result["reachable"] == true
        })
        .count();
    let recommended_fallback = if direct_failures > 0 && direct_successes == 0 {
        Some("ssh-native")
    } else {
        None
    };
    let selected_reason = if recommended_fallback.is_some() {
        "all probed direct peer transports failed; SSH fallback is recommended before starting the route"
    } else if direct_successes > 0 {
        "at least one direct peer transport was reachable before route start"
    } else {
        "no failing direct peer transport was observed before route start"
    };
    let repair_hint = if recommended_fallback.is_some() {
        "use ssh-native fallback, or publish a peer endpoint reachable from this client"
    } else if candidate_failures.is_empty() {
        "none"
    } else {
        "publish a reachable peer endpoint, adjust firewall/NAT, or switch to an SSH fallback transport"
    };
    forward.preflight_recommended_fallback = recommended_fallback.map(str::to_string);
    forward.preflight_selected_reason = Some(selected_reason.to_string());
    forward.preflight_repair_hint = Some(repair_hint.to_string());
    forward.preflight_candidate_failures = candidate_failures.clone();

    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "preflight".to_string(),
            json!({
                "kind": "local-direct-transport-probe",
                "timeout_ms": timeout.as_millis(),
                "results": results,
                "candidate_failures": candidate_failures,
                "recommended_fallback": recommended_fallback,
                "selected_reason": selected_reason,
                "repair_hint": repair_hint,
            }),
        );
    }
    refresh_decision_chain(plan);
}

pub(crate) fn apply_local_forward_fallback(
    forward: &mut cli::NodeForwardArgs,
    plan: &mut Value,
) -> Option<String> {
    let recommended = plan
        .pointer("/preflight/recommended_fallback")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let Some(recommended) = recommended else {
        return None;
    };
    let source = forward
        .transport_selection_source
        .as_deref()
        .unwrap_or("unknown");
    let may_override = forward.remote_transport == cli::RemoteTransport::Auto
        || matches!(source, "topology" | "benchmark-tuned default");
    if !may_override {
        return None;
    }
    if recommended != "ssh-native" && recommended != "ssh-direct-tcpip" {
        return None;
    }
    forward.remote_transport = cli::RemoteTransport::SshNative;
    let reason =
        "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
            .to_string();
    forward.transport_selection_source = Some("route-preflight".to_string());
    forward.transport_selection_reason = Some(reason.clone());
    if let Some(object) = plan.as_object_mut() {
        object.insert("selected_transport".to_string(), json!("ssh-native"));
        object.insert(
            "transport_selection_source".to_string(),
            json!("route-preflight"),
        );
        object.insert(
            "transport_selection_reason".to_string(),
            json!(reason.clone()),
        );
        object.insert("ssh_mode".to_string(), json!("native-direct-tcpip"));
        object.insert(
            "ssh_mode_reason".to_string(),
            ssh_mode_reason(cli::RemoteTransport::SshNative),
        );
        object.insert(
            "ssh_data_plane_reason".to_string(),
            ssh_data_plane_reason(
                cli::RemoteTransport::SshNative,
                forward.transport_selection_source.as_deref(),
            ),
        );
        object.insert(
            "ssh_session_pool_size".to_string(),
            json!(forward.ssh_session_pool_size.unwrap_or(1)),
        );
        object.insert(
            "ssh_session_pool_source".to_string(),
            json!(
                forward
                    .ssh_session_pool_source
                    .as_deref()
                    .unwrap_or("unknown")
            ),
        );
        object.insert(
            "ssh_session_pool_reason".to_string(),
            json!(
                forward
                    .ssh_session_pool_reason
                    .as_deref()
                    .unwrap_or("unknown")
            ),
        );
        object.insert(
            "ssh_session_pool_warning".to_string(),
            json!(forward.ssh_session_pool_warning.as_deref()),
        );
        object.insert("fallback_reason".to_string(), json!(reason.clone()));
        object.insert(
            "next_action".to_string(),
            json!("using ssh-native fallback; no user action required"),
        );
    }
    refresh_decision_chain(plan);
    Some(reason)
}

async fn probe_quic_endpoint(
    forward: &cli::NodeForwardArgs,
    addr: SocketAddr,
    timeout: Duration,
) -> Value {
    if addr.ip().is_loopback() {
        return json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "loopback QUIC endpoint is local to the caller; probing it here would not prove reachability to the peer daemon",
        });
    }
    let Some(ca) = forward.remote_ca.as_deref() else {
        return json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "QUIC handshake probe requires --remote-ca or profile remote_ca",
        });
    };
    if forward.remote_client_cert.is_some() || forward.remote_client_key.is_some() {
        return json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "QUIC mTLS probing is not implemented yet; use TLS/TCP probing for mTLS routes",
        });
    }

    let roots = match peer_transport::load_cert_chain(ca) {
        Ok(roots) => roots,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("failed to load QUIC probe CA {}: {err:#}", ca.display()),
            });
        }
    };
    let mut endpoint = match quinn::Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0))) {
        Ok(endpoint) => endpoint,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("failed to create QUIC probe endpoint: {err:#}"),
            });
        }
    };
    let quic_options = match peer_transport::QuicTransportOptions::new(
        forward.quic_max_bidi_streams,
        forward.quic_stream_receive_window,
        forward.quic_receive_window,
        forward.quic_keep_alive_interval_secs,
        forward.quic_idle_timeout_secs,
    ) {
        Ok(options) => options,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("invalid QUIC probe transport config: {err:#}"),
            });
        }
    };
    match peer_transport::quic_client_config(roots, quic_options) {
        Ok(config) => endpoint.set_default_client_config(config),
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("failed to build QUIC probe client config: {err:#}"),
            });
        }
    }

    let connecting = match endpoint.connect(addr, &forward.remote_name) {
        Ok(connecting) => connecting,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "connect-request-failed",
                "message": err.to_string(),
            });
        }
    };
    let connection = match time::timeout(timeout, connecting).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(err)) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "handshake-failed",
                "message": err.to_string(),
            });
        }
        Err(_) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "timeout",
                "message": format!("QUIC handshake timed out after {} ms", timeout.as_millis()),
            });
        }
    };
    let (send, recv) = match time::timeout(timeout, connection.open_bi()).await {
        Ok(Ok(streams)) => streams,
        Ok(Err(err)) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "stream-open-failed",
                "message": err.to_string(),
            });
        }
        Err(_) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "timeout",
                "message": format!("QUIC bidirectional stream open timed out after {} ms", timeout.as_millis()),
            });
        }
    };
    let mut stream = quic_stream::QuicBiStream::with_lifetime(send, recv, connection, endpoint);
    match time::timeout(
        timeout,
        peer_transport::client_handshake(
            &mut stream,
            "route-preflight",
            peer_transport::PeerProtocol::Quic,
        ),
    )
    .await
    {
        Ok(Ok(welcome)) => {
            stream.finish();
            json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": true,
                "status": "reachable",
                "message": format!("QUIC handshake succeeded before route start; remote node {}", welcome.node),
            })
        }
        Ok(Err(err)) => json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "peer-handshake-failed",
            "message": format!("{err:#}"),
        }),
        Err(_) => json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "timeout",
            "message": format!("QUIC peer handshake timed out after {} ms", timeout.as_millis()),
        }),
    }
}

async fn probe_tcp_endpoint(protocol: &str, addr: SocketAddr, timeout: Duration) -> Value {
    if addr.ip().is_loopback() {
        return json!({
            "protocol": protocol,
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "loopback endpoint is local to the caller; probing it here would not prove reachability to the peer daemon",
        });
    }

    match time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            json!({
                "protocol": protocol,
                "endpoint": addr.to_string(),
                "reachable": true,
                "status": "reachable",
                "message": "TCP connect succeeded before route start",
            })
        }
        Ok(Err(err)) => json!({
            "protocol": protocol,
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "connect-failed",
            "message": err.to_string(),
        }),
        Err(_) => json!({
            "protocol": protocol,
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "timeout",
            "message": format!("TCP connect timed out after {} ms", timeout.as_millis()),
        }),
    }
}

fn candidate_failures(results: &[Value]) -> Vec<Value> {
    results
        .iter()
        .filter(|result| is_direct_probe_protocol(result["protocol"].as_str()))
        .filter(|result| result["reachable"] == false)
        .cloned()
        .collect()
}

fn is_direct_probe_protocol(protocol: Option<&str>) -> bool {
    matches!(protocol, Some("quic" | "tls-tcp" | "plain-tcp"))
}

fn route_runtime_plan(
    reconnect_delay_secs: u64,
    reconnect_max_delay_secs: u64,
    connect_timeout_secs: u64,
    transport_pool_size: usize,
    transport_pool_source: Option<&str>,
    transport_pool_reason: Option<&str>,
    pool_policy: Option<&str>,
    workload_hint: Option<&str>,
    no_reconnect: bool,
) -> Value {
    json!({
        "reconnect_delay_secs": reconnect_delay_secs,
        "reconnect_max_delay_secs": reconnect_max_delay_secs,
        "connect_timeout_secs": connect_timeout_secs,
        "transport_pool_size": transport_pool_size,
        "transport_pool_source": transport_pool_source.unwrap_or("implicit").to_string(),
        "transport_pool_reason": transport_pool_reason.unwrap_or("implicit single-worker default").to_string(),
        "pool_policy": pool_policy.unwrap_or("large").to_string(),
        "workload_hint": workload_hint.unwrap_or("large").to_string(),
        "no_reconnect": no_reconnect,
    })
}

fn refresh_decision_chain(plan: &mut Value) {
    let selected_transport = plan
        .get("selected_transport")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let selection_source = plan
        .get("transport_selection_source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let selection_reason = plan
        .get("transport_selection_reason")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let fallback_reason = plan.get("fallback_reason").cloned().unwrap_or(Value::Null);
    let next_action = plan
        .get("next_action")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let direct_transport_policy = plan
        .get("direct_transport_policy")
        .cloned()
        .unwrap_or(Value::Null);
    let direct_transport_policy_reason = plan
        .get("direct_transport_policy_reason")
        .cloned()
        .unwrap_or(Value::Null);
    let tls_peer_auth_mode = plan
        .get("tls_peer_auth_mode")
        .cloned()
        .unwrap_or(Value::Null);
    let runtime = plan.get("runtime");
    let topology = plan.get("topology");
    let preflight = plan.get("preflight");
    let topology_class = route_topology_class(preflight, topology);
    let decision_chain = json!({
        "preflight": {
            "kind": preflight.and_then(|value| value.get("kind")).cloned().unwrap_or(Value::Null),
            "recommended_fallback": preflight.and_then(|value| value.get("recommended_fallback")).cloned().unwrap_or(Value::Null),
            "selected_reason": preflight.and_then(|value| value.get("selected_reason")).cloned().unwrap_or(Value::Null),
            "repair_hint": preflight.and_then(|value| value.get("repair_hint")).cloned().unwrap_or(Value::Null),
            "candidate_failures": preflight.and_then(|value| value.get("candidate_failures")).cloned().unwrap_or_else(|| json!([])),
        },
        "topology": {
            "class": topology_class,
            "ssh_jump_chain": topology.and_then(|value| value.get("ssh_jump_chain")).cloned().unwrap_or_else(|| json!([])),
            "direct_private_candidates": topology.and_then(|value| value.get("direct_private_candidates")).cloned().unwrap_or_else(|| json!([])),
        },
        "policy": {
            "direct_transport_policy": direct_transport_policy,
            "direct_transport_policy_reason": direct_transport_policy_reason,
            "tls_peer_auth_mode": tls_peer_auth_mode,
            "ssh_data_plane_reason": plan
                .get("ssh_data_plane_reason")
                .cloned()
                .unwrap_or(Value::Null),
            "explicit_user_override": matches!(selection_source, "cli" | "profile"),
            "selection_source": selection_source,
        },
        "workload": {
            "hint": runtime.and_then(|value| value.get("workload_hint")).cloned().unwrap_or(Value::Null),
            "pool_policy": runtime.and_then(|value| value.get("pool_policy")).cloned().unwrap_or(Value::Null),
            "transport_pool_size": runtime.and_then(|value| value.get("transport_pool_size")).cloned().unwrap_or(Value::Null),
        },
        "selected_transport": selected_transport,
        "selected_reason": selection_reason,
        "fallback_reason": fallback_reason,
        "next_action": next_action,
    });
    if let Some(object) = plan.as_object_mut() {
        object.insert("decision_chain".to_string(), decision_chain);
    }
}

fn route_topology_class(preflight: Option<&Value>, topology: Option<&Value>) -> &'static str {
    let direct_reachable = preflight
        .and_then(|value| value.get("results"))
        .and_then(Value::as_array)
        .map(|results| {
            results.iter().any(|result| {
                is_direct_probe_protocol(result.get("protocol").and_then(Value::as_str))
                    && result.get("reachable") == Some(&Value::Bool(true))
            })
        })
        .unwrap_or(false);
    if direct_reachable {
        return "direct-reachable";
    }

    let recommended_fallback = preflight
        .and_then(|value| value.get("recommended_fallback"))
        .and_then(Value::as_str);
    if recommended_fallback.is_some() {
        return "ssh-only";
    }

    let has_jump = topology
        .and_then(|value| value.get("ssh_jump_chain"))
        .and_then(Value::as_array)
        .map(|chain| !chain.is_empty())
        .unwrap_or(false);
    let has_direct_candidates = topology
        .and_then(|value| value.get("direct_private_candidates"))
        .and_then(Value::as_array)
        .map(|candidates| !candidates.is_empty())
        .unwrap_or(false);
    if has_jump && has_direct_candidates {
        return "mixed";
    }
    if has_direct_candidates {
        return "unknown-direct";
    }
    "ssh-reachable"
}

fn transport_candidates(forward: &cli::NodeForwardArgs) -> Vec<String> {
    let mut candidates = Vec::new();
    if forward.remote_quic.is_some() {
        candidates.push("quic".to_string());
    }
    if forward.remote_tls.is_some() {
        candidates.push("tls-tcp".to_string());
    }
    if forward.allow_plain_tcp {
        candidates.push("plain-tcp".to_string());
    }
    candidates.push("ssh-native".to_string());
    candidates.push("ssh-direct-tcpip".to_string());
    candidates.push("ssh-exec".to_string());
    candidates
}

fn topology_hint(args: &cli::RouteArgs, forward: &cli::NodeForwardArgs) -> Value {
    let ssh_target = ssh_client::resolve_route_target(args);
    let (ssh_host, ssh_jump_chain) = match ssh_target {
        Ok(target) => (
            Some(format!("{}:{}", target.host, target.port)),
            target
                .jumps
                .into_iter()
                .map(|jump| format!("{}@{}:{}", jump.user, jump.host, jump.port))
                .collect::<Vec<_>>(),
        ),
        Err(err) => {
            return json!({
                "ssh_target": Value::Null,
                "ssh_jump_chain": [],
                "direct_private_candidates": direct_private_candidates(forward),
                "warning": format!("failed to resolve SSH target for topology hint: {err}"),
            });
        }
    };
    let direct_private_candidates = direct_private_candidates(forward);
    let warning = if !ssh_jump_chain.is_empty() && !direct_private_candidates.is_empty() {
        Some(
            "SSH target uses ProxyJump; direct QUIC/TLS/plain peer endpoints do not automatically traverse the SSH jump path and may be unreachable. Prefer SSH fallback or a reachable peer endpoint."
                .to_string(),
        )
    } else if direct_private_candidates
        .iter()
        .any(|candidate| candidate.ends_with("127.0.0.1") || candidate.contains("127.0.0.1:"))
    {
        Some(
            "direct peer endpoint is loopback; it is reachable only from the same machine or through SSH direct-tcpip fallback"
                .to_string(),
        )
    } else {
        None
    };
    json!({
        "ssh_target": ssh_host,
        "ssh_jump_chain": ssh_jump_chain,
        "direct_private_candidates": direct_private_candidates,
        "warning": warning,
    })
}

fn direct_private_candidates(forward: &cli::NodeForwardArgs) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(addr) = forward.remote_quic {
        candidates.push(format!("quic://{addr}"));
    }
    if let Some(addr) = forward.remote_tls {
        candidates.push(format!("tls-tcp://{addr}"));
    }
    if forward.allow_plain_tcp {
        candidates.push(format!("plain-tcp://{}", forward.remote_tcp));
    }
    candidates
}

fn remote_transport_name(transport: cli::RemoteTransport) -> &'static str {
    match transport {
        cli::RemoteTransport::Auto => "auto",
        cli::RemoteTransport::SshNative => "ssh-native",
        cli::RemoteTransport::QuicNative => "quic-native",
        cli::RemoteTransport::Quic => "quic",
        cli::RemoteTransport::TlsTcp => "tls-tcp",
        cli::RemoteTransport::PlainTcp => "plain-tcp",
        cli::RemoteTransport::Exec => "ssh-exec",
        cli::RemoteTransport::Tcp => "ssh-direct-tcpip",
    }
}

fn direct_transport_policy(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::TlsTcp => json!("production_direct"),
        cli::RemoteTransport::PlainTcp => json!("lab_baseline"),
        cli::RemoteTransport::Quic | cli::RemoteTransport::QuicNative => json!("experimental"),
        _ => Value::Null,
    }
}

fn direct_transport_policy_reason(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::TlsTcp => json!(
            "TLS/TCP SPX is the production direct baseline because it keeps the stable SPX data plane while adding peer encryption and certificate identity"
        ),
        cli::RemoteTransport::PlainTcp => json!(
            "Plain TCP SPX is a lab or explicitly trusted baseline only; it is not selected as the production default because the data path is not encrypted"
        ),
        cli::RemoteTransport::Quic | cli::RemoteTransport::QuicNative => json!(
            "QUIC direct transport remains experimental until throughput and recovery behavior close the gap with TLS/TCP SPX"
        ),
        _ => Value::Null,
    }
}

fn tls_peer_auth_mode<T, U>(
    transport: cli::RemoteTransport,
    client_cert: Option<T>,
    client_key: Option<U>,
) -> Value {
    if !matches!(transport, cli::RemoteTransport::TlsTcp) {
        return Value::Null;
    }
    match (client_cert.is_some(), client_key.is_some()) {
        (true, true) => json!("mutual_tls"),
        (false, false) => json!("server_auth"),
        _ => json!("invalid_client_auth_config"),
    }
}

fn ssh_mode_name(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::SshNative => json!("native-direct-tcpip"),
        cli::RemoteTransport::Tcp => json!("spx-over-ssh-direct"),
        cli::RemoteTransport::Exec => json!("ssh-exec-helper"),
        _ => Value::Null,
    }
}

fn ssh_mode_reason(transport: cli::RemoteTransport) -> Value {
    match transport {
        cli::RemoteTransport::SshNative => json!(
            "ssh-native opens russh direct-tcpip channels to each requested target; use it for simple SSH-only local egress because it avoids remote daemon and SPX framed data-plane overhead"
        ),
        cli::RemoteTransport::Tcp => json!(
            "spx-over-ssh-direct opens SSH direct-tcpip to the remote daemon transport and keeps SPX daemon semantics; use it when remote daemon policy, token auth, route restore, or SPX UDP behavior is required"
        ),
        cli::RemoteTransport::Exec => json!(
            "ssh-exec-helper starts a temporary remote helper over SSH; keep it as a compatibility path when no persistent remote daemon transport is available"
        ),
        _ => Value::Null,
    }
}

fn ssh_data_plane_reason(transport: cli::RemoteTransport, selection_source: Option<&str>) -> Value {
    if matches!(selection_source, Some("cli" | "profile")) {
        return match transport {
            cli::RemoteTransport::SshNative
            | cli::RemoteTransport::Tcp
            | cli::RemoteTransport::Exec => json!("explicit_user_choice"),
            _ => Value::Null,
        };
    }
    match transport {
        cli::RemoteTransport::SshNative => json!("simple_egress"),
        cli::RemoteTransport::Tcp => json!("daemon_policy_required"),
        cli::RemoteTransport::Exec => json!("ssh_exec_compatibility"),
        _ => Value::Null,
    }
}

fn local_peer_addr(args: &cli::RouteArgs, config: &config::AppConfig) -> Result<SocketAddr> {
    if let Some(addr) = args.local_peer {
        return Ok(addr);
    }
    let Some(addr) = config.daemon.transport_listen else {
        bail!(
            "--direction remote-uses-local needs --local-peer or [daemon].transport_listen; run `ssh_proxy service install` first"
        );
    };
    if addr.ip().is_loopback() {
        bail!(
            "local daemon transport {addr} is loopback-only; pass --local-peer <reachable-ip:port>, or use a public/TLS/QUIC relay route when this machine is behind NAT"
        );
    }
    Ok(addr)
}

pub(crate) fn route_id(args: &cli::RouteArgs, prefix: &str) -> String {
    args.id
        .clone()
        .unwrap_or_else(|| format!("{prefix}:{}:{}", args.target, args.port))
}

fn parse_deploy(value: &str) -> Result<cli::DeployMode> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::DeployMode::Auto),
        "always" => Ok(cli::DeployMode::Always),
        "never" => Ok(cli::DeployMode::Never),
        other => bail!("invalid deploy value {other:?}"),
    }
}

fn parse_remote_os(value: &str) -> Result<cli::RemoteOs> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::RemoteOs::Auto),
        "unix" | "linux" | "macos" => Ok(cli::RemoteOs::Unix),
        "windows" => Ok(cli::RemoteOs::Windows),
        other => bail!("invalid remote_os value {other:?}"),
    }
}

fn parse_remote_transport(value: &str) -> Result<cli::RemoteTransport> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(cli::RemoteTransport::Auto),
        "quic-native" | "quic_native" | "native-quic" | "native_quic" => {
            Ok(cli::RemoteTransport::QuicNative)
        }
        "ssh-native" | "ssh_native" | "native-ssh" | "native_ssh" => {
            Ok(cli::RemoteTransport::SshNative)
        }
        "quic" => Ok(cli::RemoteTransport::Quic),
        "tls-tcp" | "tls_tcp" | "tls" => Ok(cli::RemoteTransport::TlsTcp),
        "plain-tcp" | "plain_tcp" | "direct-tcp" | "direct_tcp" => {
            Ok(cli::RemoteTransport::PlainTcp)
        }
        "exec" => Ok(cli::RemoteTransport::Exec),
        "tcp" => Ok(cli::RemoteTransport::Tcp),
        other => bail!("invalid remote_transport value {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route_args(direction: cli::RouteDirection) -> cli::RouteArgs {
        cli::RouteArgs {
            target: "peer".to_string(),
            direction,
            connect_mode: cli::RouteConnectMode::Auto,
            port: 18080,
            bind: "127.0.0.1".parse().unwrap(),
            tcp_target: None,
            endpoint: "tcp://127.0.0.1:1".to_string(),
            token: None,
            ssh_args: Vec::new(),
            user: None,
            ssh_port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            deploy: cli::DeployMode::Auto,
            remote_os: cli::RemoteOs::Auto,
            remote_transport: cli::RemoteTransport::Auto,
            remote_tcp: None,
            remote_control: None,
            remote_quic: None,
            remote_tls: None,
            remote_ca: None,
            remote_name: "localhost".to_string(),
            remote_token: None,
            egress_proxy: None,
            reconnect_delay_secs: None,
            reconnect_max_delay_secs: None,
            connect_timeout_secs: None,
            quic_max_bidi_streams: None,
            quic_stream_receive_window: None,
            quic_receive_window: None,
            quic_keep_alive_interval_secs: None,
            quic_idle_timeout_secs: None,
            transport_pool_size: None,
            workload_hint: None,
            ssh_session_pool_size: None,
            no_reconnect: false,
            local_peer: None,
            allow_plain_tcp: false,
            id: None,
            volatile: false,
            dry_run: true,
            explain: false,
            json: false,
        }
    }

    #[test]
    fn remote_uses_local_requires_reachable_local_peer() {
        let mut args = route_args(cli::RouteDirection::RemoteUsesLocal);
        args.connect_mode = cli::RouteConnectMode::Direct;
        let config = config::AppConfig {
            daemon: config::DaemonConfig {
                transport_listen: Some("127.0.0.1:19080".parse().unwrap()),
                ..Default::default()
            },
            ..Default::default()
        };

        let err = local_peer_addr(&args, &config).unwrap_err().to_string();

        assert!(err.contains("loopback-only"));
    }

    #[test]
    fn auto_remote_uses_local_falls_back_to_reverse_link_behind_nat() {
        let args = route_args(cli::RouteDirection::RemoteUsesLocal);
        let config = config::AppConfig {
            daemon: config::DaemonConfig {
                transport_listen: Some("127.0.0.1:19080".parse().unwrap()),
                ..Default::default()
            },
            ..Default::default()
        };

        let plan = remote_use_decision(&args, &config).unwrap().plan;

        assert!(matches!(plan, RemoteUsePlan::ReverseLink));
    }

    #[test]
    fn auto_remote_uses_local_explains_reverse_link_fallback() {
        let args = route_args(cli::RouteDirection::RemoteUsesLocal);
        let config = config::AppConfig {
            daemon: config::DaemonConfig {
                transport_listen: Some("127.0.0.1:19080".parse().unwrap()),
                ..Default::default()
            },
            ..Default::default()
        };

        let decision = remote_use_decision(&args, &config).unwrap();
        let reverse = node_reverse_from_route(&args, &config).unwrap();
        let plan = remote_uses_local_reverse_link_plan(
            &args,
            "reverse-id",
            &reverse,
            decision.fallback_reason.as_deref(),
        );

        assert!(matches!(decision.plan, RemoteUsePlan::ReverseLink));
        assert_eq!(plan["mode"], "reverse-link");
        assert_eq!(plan["owner"], "local");
        assert_eq!(plan["selected_transport"], "ssh-reverse-link");
        assert!(
            plan["fallback_reason"]
                .as_str()
                .expect("fallback reason")
                .contains("loopback-only")
        );
        assert_eq!(
            plan["next_action"],
            "set --local-peer <reachable-ip:port> for direct mode"
        );
    }

    #[test]
    fn direct_remote_uses_local_plan_lists_reachable_peer() {
        let mut args = route_args(cli::RouteDirection::RemoteUsesLocal);
        args.local_peer = Some("192.0.2.8:19080".parse().unwrap());
        args.allow_plain_tcp = true;
        let config = config::AppConfig::default();
        let token = "token".to_string();
        let local_peer = args.local_peer.unwrap();

        let host_args = remote_direct_host_args(&args, &config, local_peer, token).unwrap();
        let cli::HostCommand::NodeForward(forward) = &host_args.command else {
            panic!("expected node forward");
        };
        let plan = remote_uses_local_direct_plan(
            &args,
            forward.id.as_deref().unwrap(),
            forward,
            local_peer,
        );

        assert_eq!(plan["mode"], "direct");
        assert_eq!(plan["owner"], "remote");
        assert_eq!(plan["egress"]["reachable_peer"], "192.0.2.8:19080");
        assert!(
            plan["transport_candidates"]
                .as_array()
                .unwrap()
                .contains(&serde_json::Value::String("plain-tcp".to_string()))
        );
    }

    #[test]
    fn route_plan_warns_when_direct_transport_meets_ssh_jump() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.jump = vec!["jump.example.com".to_string()];
        args.remote_quic = Some("192.0.2.71:19083".parse().unwrap());
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert!(
            plan["topology"]["ssh_jump_chain"][0]
                .as_str()
                .expect("jump chain")
                .contains("jump.example.com")
        );
        assert!(
            plan["topology"]["warning"]
                .as_str()
                .expect("topology warning")
                .contains("ProxyJump")
        );
    }

    #[tokio::test]
    async fn preflight_probe_skips_loopback_tls_tcp_candidate() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_tls = Some(addr);
        let config = config::AppConfig::default();
        let mut forward =
            node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let mut plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        add_local_transport_probe_results(&mut plan, &mut forward).await;

        let results = plan["preflight"]["results"].as_array().unwrap();
        let tls = results
            .iter()
            .find(|result| result["protocol"] == "tls-tcp")
            .expect("tls probe result");
        assert_eq!(tls["status"], "skipped");
        server.abort();
    }

    #[tokio::test]
    async fn preflight_probe_reports_unreachable_non_loopback_candidate() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_tls = Some("192.0.2.1:9".parse().unwrap());
        let config = config::AppConfig::default();
        let mut forward =
            node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let mut plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        add_local_transport_probe_results(&mut plan, &mut forward).await;

        let results = plan["preflight"]["results"].as_array().unwrap();
        let tls = results
            .iter()
            .find(|result| result["protocol"] == "tls-tcp")
            .expect("tls probe result");
        assert_eq!(tls["reachable"], false);
        assert_eq!(plan["preflight"]["recommended_fallback"], "ssh-native");
        assert_eq!(
            plan["preflight"]["selected_reason"],
            "all probed direct peer transports failed; SSH fallback is recommended before starting the route"
        );
        assert_eq!(
            plan["preflight"]["repair_hint"],
            "use ssh-native fallback, or publish a peer endpoint reachable from this client"
        );
        assert_eq!(
            plan["preflight"]["candidate_failures"][0]["protocol"],
            "tls-tcp"
        );
        apply_local_forward_fallback(&mut forward, &mut plan);
        assert_eq!(plan["selected_transport"], "ssh-native");
        assert_eq!(plan["ssh_mode"], "native-direct-tcpip");
        assert_eq!(plan["ssh_session_pool_size"], 2);
        assert_eq!(
            plan["fallback_reason"],
            "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
        );
        assert_eq!(
            plan["next_action"],
            "using ssh-native fallback; no user action required"
        );
        let request = route_start_request_with_reason(
            "route-id",
            forward.clone(),
            true,
            Some("direct private transport preflight failed; selected SSH native direct-tcpip fallback".to_string()),
        );
        assert_eq!(
            request["proxy"]["preflight_selected_reason"],
            "all probed direct peer transports failed; SSH fallback is recommended before starting the route"
        );
        assert_eq!(
            request["proxy"]["preflight_repair_hint"],
            "use ssh-native fallback, or publish a peer endpoint reachable from this client"
        );
        assert_eq!(
            request["proxy"]["preflight_candidate_failures"][0]["protocol"],
            "tls-tcp"
        );
    }

    #[tokio::test]
    async fn preflight_quic_candidate_is_handshake_probed_when_configured_or_explained() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_quic = Some("192.0.2.1:19083".parse().unwrap());
        let config = config::AppConfig::default();
        let mut forward =
            node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let mut plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        add_local_transport_probe_results(&mut plan, &mut forward).await;

        let results = plan["preflight"]["results"].as_array().unwrap();
        let quic = results
            .iter()
            .find(|result| result["protocol"] == "quic")
            .expect("quic probe result");
        assert_eq!(quic["status"], "skipped");
        assert!(
            quic["message"]
                .as_str()
                .expect("quic message")
                .contains("remote-ca")
        );
    }

    #[tokio::test]
    async fn preflight_fallback_switches_auto_transport_to_ssh_native() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_tls = Some("192.0.2.1:9".parse().unwrap());
        let config = config::AppConfig::default();
        let mut forward =
            node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let id = forward.id.as_deref().unwrap().to_string();
        let mut plan = local_uses_remote_plan(&args, &id, &forward);

        add_local_transport_probe_results(&mut plan, &mut forward).await;
        let fallback = apply_local_forward_fallback(&mut forward, &mut plan);

        assert_eq!(
            fallback.as_deref(),
            Some(
                "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
            )
        );
        assert_eq!(forward.remote_transport, cli::RemoteTransport::SshNative);
        assert_eq!(plan["selected_transport"], "ssh-native");
        assert_eq!(plan["ssh_mode"], "native-direct-tcpip");
        assert_eq!(plan["ssh_data_plane_reason"], "simple_egress");
        assert_eq!(
            plan["decision_chain"]["policy"]["ssh_data_plane_reason"],
            "simple_egress"
        );
        assert_eq!(
            plan["fallback_reason"],
            "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
        );
        assert_eq!(
            plan["next_action"],
            "using ssh-native fallback; no user action required"
        );
    }

    #[test]
    fn reverse_link_route_is_local_daemon_reverse_spec() {
        let args = route_args(cli::RouteDirection::RemoteUsesLocal);
        let config = config::AppConfig::default();

        let reverse = node_reverse_from_route(&args, &config).unwrap();
        let request = reverse_route_start_request("reverse-id", reverse, true);

        assert_eq!(request["direction"], "reverse");
        assert_eq!(request["connect_mode"], "reverse-link");
        assert_eq!(request["reverse"]["target"], "peer");
        assert_eq!(request["reverse"]["remote_listen"], "127.0.0.1:18080");
    }

    #[test]
    fn route_tcp_target_is_carried_into_plan_and_spec() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.tcp_target = Some("example.com:443".parse().unwrap());
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);
        let request = route_start_request("fixed", forward, true);

        assert_eq!(plan["listener"]["tcp_target"], "example.com:443");
        assert_eq!(request["proxy"]["tcp_target"]["host"], "example.com");
        assert_eq!(request["proxy"]["tcp_target"]["port"], 443);
    }

    #[test]
    fn route_uses_saved_target_peer_defaults() {
        let args = route_args(cli::RouteDirection::LocalUsesRemote);
        let mut config = config::AppConfig::default();
        config.profiles.insert(
            "peer".to_string(),
            config::ProxyProfile {
                remote_tcp: Some("127.0.0.1:29080".parse().unwrap()),
                remote_control: Some("127.0.0.1:29081".parse().unwrap()),
                remote_token: Some("saved-token".to_string()),
                remote_transport: Some("tcp".to_string()),
                ..Default::default()
            },
        );

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_tcp, "127.0.0.1:29080".parse().unwrap());
        assert_eq!(forward.remote_control, "127.0.0.1:29081".parse().unwrap());
        assert_eq!(forward.remote_token.as_deref(), Some("saved-token"));
        assert_eq!(forward.remote_transport, cli::RemoteTransport::Tcp);
        assert_eq!(plan["ssh_mode"], "spx-over-ssh-direct");
        assert_eq!(plan["ssh_data_plane_reason"], "explicit_user_choice");
        assert!(
            plan["ssh_mode_reason"]
                .as_str()
                .expect("ssh mode reason")
                .contains("remote daemon policy")
        );
        assert!(
            plan["ssh_mode_reason"]
                .as_str()
                .expect("ssh mode reason")
                .contains("SPX UDP")
        );
    }

    #[test]
    fn quic_native_transport_is_parseable_but_not_auto_selected() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_transport = parse_remote_transport("quic-native").unwrap();
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::QuicNative);
        assert_eq!(plan["selected_transport"], "quic-native");
        assert_eq!(plan["ssh_mode"], serde_json::Value::Null);
        assert_eq!(plan["ssh_mode_reason"], serde_json::Value::Null);
    }

    #[test]
    fn route_auto_selects_ssh_native_for_ssh_only_proxy_topology() {
        let args = route_args(cli::RouteDirection::LocalUsesRemote);
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::SshNative);
        assert_eq!(plan["selected_transport"], "ssh-native");
        assert_eq!(plan["transport_selection_source"], "topology");
        assert_eq!(plan["ssh_data_plane_reason"], "simple_egress");
        assert!(
            plan["transport_selection_reason"]
                .as_str()
                .expect("selection reason")
                .contains("SSH-only simple egress default")
        );
    }

    #[test]
    fn route_auto_prefers_tls_tcp_over_plain_for_direct_production() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_tls = Some("192.0.2.8:19082".parse().unwrap());
        args.allow_plain_tcp = true;
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::TlsTcp);
        assert_eq!(plan["selected_transport"], "tls-tcp");
        assert_eq!(plan["direct_transport_policy"], "production_direct");
        assert!(
            plan["direct_transport_policy_reason"]
                .as_str()
                .expect("policy reason")
                .contains("production direct baseline")
        );
        assert_eq!(
            plan["decision_chain"]["policy"]["direct_transport_policy"],
            "production_direct"
        );
        assert!(
            plan["decision_chain"]["policy"]["direct_transport_policy_reason"]
                .as_str()
                .expect("decision policy reason")
                .contains("certificate identity")
        );
        assert_eq!(plan["tls_peer_auth_mode"], "server_auth");
        assert_eq!(plan["transport_selection_source"], "topology");
        assert!(
            plan["transport_selection_reason"]
                .as_str()
                .expect("selection reason")
                .contains("production direct default")
        );
    }

    #[test]
    fn route_plan_reports_tls_mutual_auth_mode() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_tls = Some("192.0.2.8:19082".parse().unwrap());
        let mut config = config::AppConfig::default();
        config.defaults.remote_client_cert = Some(std::path::PathBuf::from("client.pem"));
        config.defaults.remote_client_key = Some(std::path::PathBuf::from("client-key.pem"));

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(plan["selected_transport"], "tls-tcp");
        assert_eq!(plan["direct_transport_policy"], "production_direct");
        assert_eq!(plan["tls_peer_auth_mode"], "mutual_tls");
    }

    #[test]
    fn route_auto_ignores_unsafe_plain_tcp_default_when_tls_is_available() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_tls = Some("192.0.2.8:19082".parse().unwrap());
        let mut config = config::AppConfig::default();
        config.defaults.remote_transport = Some("plain-tcp".to_string());

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::TlsTcp);
        assert_eq!(plan["selected_transport"], "tls-tcp");
        assert_eq!(plan["direct_transport_policy"], "production_direct");
        assert_eq!(plan["tls_peer_auth_mode"], "server_auth");
        assert_eq!(plan["transport_selection_source"], "topology");
        assert!(
            plan["transport_selection_reason"]
                .as_str()
                .expect("selection reason")
                .contains("production direct default")
        );
    }

    #[test]
    fn route_auto_ignores_plain_tcp_default_unless_lab_enabled() {
        let args = route_args(cli::RouteDirection::LocalUsesRemote);
        let mut config = config::AppConfig::default();
        config.defaults.remote_transport = Some("plain-tcp".to_string());

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::SshNative);
        assert_eq!(plan["selected_transport"], "ssh-native");
        assert_eq!(plan["transport_selection_source"], "topology");
        assert!(!forward.allow_plain_tcp);
    }

    #[test]
    fn route_auto_uses_plain_tcp_only_when_explicitly_allowed() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.allow_plain_tcp = true;
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::PlainTcp);
        assert_eq!(plan["selected_transport"], "plain-tcp");
        assert_eq!(plan["direct_transport_policy"], "lab_baseline");
        assert!(
            plan["direct_transport_policy_reason"]
                .as_str()
                .expect("policy reason")
                .contains("lab or explicitly trusted baseline")
        );
        assert!(plan["tls_peer_auth_mode"].is_null());
        assert_eq!(plan["transport_selection_source"], "cli");
        assert!(
            plan["transport_selection_reason"]
                .as_str()
                .expect("selection reason")
                .contains("plain TCP peer transport is enabled")
        );
    }

    #[test]
    fn route_auto_uses_plain_tcp_for_benchmark_tuned_default_only_when_allowed() {
        let args = route_args(cli::RouteDirection::LocalUsesRemote);
        let mut config = config::AppConfig::default();
        config.defaults.remote_transport = Some("plain-tcp".to_string());
        config.defaults.allow_plain_tcp = Some(true);

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.remote_transport, cli::RemoteTransport::PlainTcp);
        assert_eq!(plan["selected_transport"], "plain-tcp");
        assert_eq!(plan["direct_transport_policy"], "lab_baseline");
        assert!(plan["tls_peer_auth_mode"].is_null());
        assert_eq!(
            plan["transport_selection_source"],
            "benchmark-tuned default"
        );
        assert!(
            plan["transport_selection_reason"]
                .as_str()
                .expect("selection reason")
                .contains("lab or private trusted links")
        );
    }

    #[test]
    fn route_runtime_settings_flow_into_daemon_task_args() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.connect_timeout_secs = Some(12);
        args.transport_pool_size = Some(4);
        args.quic_max_bidi_streams = Some(512);
        args.quic_stream_receive_window = Some(4 * 1024 * 1024);
        args.quic_receive_window = Some(32 * 1024 * 1024);
        args.quic_keep_alive_interval_secs = Some(20);
        args.quic_idle_timeout_secs = Some(120);
        args.no_reconnect = true;
        let mut config = config::AppConfig::default();
        config.defaults.reconnect_delay_secs = Some(7);
        config.defaults.reconnect_max_delay_secs = Some(20);
        config.defaults.connect_timeout_secs = Some(45);

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let proxy = node_daemon::proxy_args_from_node_forward(forward.clone());
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.reconnect_delay_secs, 7);
        assert_eq!(forward.reconnect_max_delay_secs, 20);
        assert_eq!(forward.connect_timeout_secs, 12);
        assert!(forward.no_reconnect);
        assert_eq!(proxy.reconnect_delay_secs, 7);
        assert_eq!(proxy.reconnect_max_delay_secs, 20);
        assert_eq!(proxy.connect_timeout_secs, 12);
        assert_eq!(proxy.transport_pool_size, 4);
        assert_eq!(proxy.pool_policy.as_deref(), Some("explicit"));
        assert_eq!(
            proxy.workload_hint,
            Some(cli::RouteWorkloadHint::Concurrent)
        );
        assert_eq!(proxy.quic_max_bidi_streams, 512);
        assert_eq!(proxy.quic_stream_receive_window, 4 * 1024 * 1024);
        assert_eq!(proxy.quic_receive_window, 32 * 1024 * 1024);
        assert_eq!(proxy.quic_keep_alive_interval_secs, 20);
        assert_eq!(proxy.quic_idle_timeout_secs, 120);
        assert!(proxy.no_reconnect);
        assert_eq!(plan["runtime"]["connect_timeout_secs"], 12);
        assert_eq!(plan["runtime"]["reconnect_delay_secs"], 7);
        assert_eq!(plan["runtime"]["reconnect_max_delay_secs"], 20);
        assert_eq!(plan["runtime"]["transport_pool_size"], 4);
        assert_eq!(plan["runtime"]["transport_pool_source"], "command-line");
        assert_eq!(plan["runtime"]["pool_policy"], "explicit");
        assert_eq!(plan["runtime"]["workload_hint"], "concurrent");
        assert_eq!(
            plan["runtime"]["transport_pool_reason"],
            "loaded from --transport-pool-size"
        );
        assert_eq!(plan["runtime"]["no_reconnect"], true);
    }

    #[test]
    fn route_runtime_uses_default_pool_metadata() {
        let args = route_args(cli::RouteDirection::LocalUsesRemote);
        let mut config = config::AppConfig::default();
        config.defaults.transport_pool_size = Some(3);

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.transport_pool_size, 3);
        assert_eq!(forward.transport_pool_source.as_deref(), Some("defaults"));
        assert_eq!(
            forward.transport_pool_reason.as_deref(),
            Some("loaded from [defaults].transport_pool_size")
        );
        assert_eq!(plan["runtime"]["transport_pool_source"], "defaults");
        assert_eq!(plan["runtime"]["pool_policy"], "explicit");
        assert_eq!(plan["runtime"]["workload_hint"], "concurrent");
        assert_eq!(
            plan["runtime"]["transport_pool_reason"],
            "loaded from [defaults].transport_pool_size"
        );
    }

    #[test]
    fn route_runtime_uses_adaptive_pool_for_proxy_vs_fixed_target() {
        let args = route_args(cli::RouteDirection::LocalUsesRemote);
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.transport_pool_size, 4);
        assert_eq!(forward.remote_transport, cli::RemoteTransport::SshNative);
        assert_eq!(forward.transport_pool_source.as_deref(), Some("implicit"));
        assert_eq!(forward.pool_policy.as_deref(), Some("concurrent"));
        assert_eq!(
            forward.workload_hint,
            Some(cli::RouteWorkloadHint::Concurrent)
        );
        assert!(
            forward
                .transport_pool_reason
                .as_deref()
                .expect("pool reason")
                .contains("multi-flow")
        );
        assert_eq!(plan["runtime"]["transport_pool_size"], 4);
        assert_eq!(plan["runtime"]["pool_policy"], "concurrent");
        assert_eq!(plan["runtime"]["workload_hint"], "concurrent");

        let mut fixed = route_args(cli::RouteDirection::LocalUsesRemote);
        fixed.tcp_target = Some("example.com:443".parse().unwrap());
        let forward = node_forward_from_route(&fixed, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&fixed, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.transport_pool_size, 1);
        assert_eq!(forward.pool_policy.as_deref(), Some("large"));
        assert_eq!(forward.workload_hint, Some(cli::RouteWorkloadHint::Large));
        assert_eq!(plan["runtime"]["pool_policy"], "large");
        assert_eq!(plan["runtime"]["workload_hint"], "large");
        assert!(
            forward
                .transport_pool_reason
                .as_deref()
                .expect("pool reason")
                .contains("fixed --tcp-target")
        );
    }

    #[test]
    fn route_runtime_uses_workload_hint_for_pool_policy() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.workload_hint = Some(cli::RouteWorkloadHint::Large);
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.transport_pool_size, 1);
        assert_eq!(forward.pool_policy.as_deref(), Some("large"));
        assert_eq!(plan["runtime"]["pool_policy"], "large");
        assert_eq!(plan["runtime"]["workload_hint"], "large");

        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.workload_hint = Some(cli::RouteWorkloadHint::Mixed);
        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.transport_pool_size, 4);
        assert_eq!(forward.pool_policy.as_deref(), Some("mixed"));
        assert_eq!(plan["runtime"]["pool_policy"], "mixed");
        assert_eq!(plan["runtime"]["workload_hint"], "mixed");
    }

    #[test]
    fn ssh_native_plan_uses_independent_session_pool_size() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_transport = cli::RemoteTransport::SshNative;
        args.transport_pool_size = Some(8);
        args.ssh_session_pool_size = Some(3);
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.transport_pool_size, 8);
        assert_eq!(forward.ssh_session_pool_size, Some(3));
        assert_eq!(
            forward.ssh_session_pool_source.as_deref(),
            Some("command-line")
        );
        assert_eq!(plan["selected_transport"], "ssh-native");
        assert_eq!(plan["ssh_session_pool_size"], 3);
        assert_eq!(plan["ssh_session_pool_source"], "command-line");
        assert_eq!(plan["ssh_data_plane_reason"], "explicit_user_choice");
        assert_eq!(
            plan["ssh_session_pool_reason"],
            "loaded from --ssh-session-pool-size"
        );
        assert_eq!(plan["ssh_mode"], "native-direct-tcpip");
    }

    #[test]
    fn ssh_native_implicit_session_pool_uses_workload_defaults() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_transport = cli::RemoteTransport::SshNative;
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.ssh_session_pool_size, Some(2));
        assert_eq!(plan["ssh_session_pool_size"], 2);
        assert_eq!(plan["ssh_session_pool_source"], "implicit");
        assert!(
            plan["ssh_session_pool_reason"]
                .as_str()
                .expect("pool reason")
                .contains("two-session")
        );

        args.tcp_target = Some("example.com:443".parse().unwrap());
        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.ssh_session_pool_size, Some(1));
        assert_eq!(plan["ssh_session_pool_size"], 1);
        assert_eq!(plan["ssh_session_pool_source"], "implicit");
        assert!(
            plan["ssh_session_pool_reason"]
                .as_str()
                .expect("pool reason")
                .contains("single-session")
        );
    }

    #[test]
    fn ssh_native_session_pool_above_two_is_annotated_not_clamped_when_explicit() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_transport = cli::RemoteTransport::SshNative;
        args.ssh_session_pool_size = Some(8);
        let config = config::AppConfig::default();

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.ssh_session_pool_size, Some(8));
        assert_eq!(plan["ssh_session_pool_size"], 8);
        assert!(
            plan["ssh_session_pool_warning"]
                .as_str()
                .expect("pool warning")
                .contains("above 2")
        );
    }

    #[test]
    fn ssh_native_defaults_above_two_are_capped_but_profile_can_override() {
        let mut args = route_args(cli::RouteDirection::LocalUsesRemote);
        args.remote_transport = cli::RemoteTransport::SshNative;
        let mut config = config::AppConfig::default();
        config.defaults.ssh_session_pool_size = Some(8);

        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.ssh_session_pool_size, Some(2));
        assert_eq!(forward.ssh_session_pool_source.as_deref(), Some("defaults"));
        assert_eq!(plan["ssh_session_pool_size"], 2);
        assert!(
            plan["ssh_session_pool_reason"]
                .as_str()
                .expect("pool reason")
                .contains("capped to pool=2")
        );
        assert!(
            plan["ssh_session_pool_warning"]
                .as_str()
                .expect("pool warning")
                .contains("defaults above 2")
        );

        config.profiles.insert(
            "peer".to_string(),
            config::ProxyProfile {
                ssh_session_pool_size: Some(4),
                ..Default::default()
            },
        );
        let forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
        let plan = local_uses_remote_plan(&args, forward.id.as_deref().unwrap(), &forward);

        assert_eq!(forward.ssh_session_pool_size, Some(4));
        assert_eq!(forward.ssh_session_pool_source.as_deref(), Some("profile"));
        assert_eq!(plan["ssh_session_pool_size"], 4);
        assert!(
            plan["ssh_session_pool_warning"]
                .as_str()
                .expect("pool warning")
                .contains("above 2")
        );
    }
}
