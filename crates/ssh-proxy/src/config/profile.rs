use std::net::SocketAddr;

use anyhow::{Result, bail};

use crate::cli;

use super::{AppConfig, ProxyProfile};

pub(super) fn default_proxy_args(target: String) -> cli::ProxyArgs {
    cli::ProxyArgs {
        target,
        listen: SocketAddr::from(([127, 0, 0, 1], 1080)),
        tcp_target: None,
        ssh_args: Vec::new(),
        ssh_command: None,
        user: None,
        port: None,
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
        remote_tcp: SocketAddr::from(([127, 0, 0, 1], 19080)),
        remote_control: SocketAddr::from(([127, 0, 0, 1], 19081)),
        remote_quic: None,
        allow_plain_tcp: false,
        remote_tls: None,
        remote_ca: None,
        remote_name: "localhost".to_string(),
        remote_client_cert: None,
        remote_client_key: None,
        remote_token: None,
        egress_proxy: None,
        reconnect_delay_secs: 5,
        reconnect_max_delay_secs: 60,
        connect_timeout_secs: 30,
        transport_pool_size: 1,
        pool_policy: None,
        workload_hint: None,
        quic_max_bidi_streams: crate::peer_transport::QUIC_MAX_BIDI_STREAMS,
        quic_stream_receive_window: crate::peer_transport::QUIC_STREAM_RECEIVE_WINDOW,
        quic_receive_window: crate::peer_transport::QUIC_RECEIVE_WINDOW,
        quic_keep_alive_interval_secs: crate::peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS,
        quic_idle_timeout_secs: crate::peer_transport::QUIC_IDLE_TIMEOUT_SECS,
        ssh_session_pool_size: None,
        ssh_session_pool_source: None,
        ssh_session_pool_reason: None,
        ssh_session_pool_warning: None,
        transport_pool_source: None,
        transport_pool_reason: None,
        transport_selection_source: None,
        transport_selection_reason: None,
        preflight_recommended_fallback: None,
        preflight_selected_reason: None,
        preflight_repair_hint: None,
        preflight_candidate_failures: Vec::new(),
        no_reconnect: false,
        control_listen: None,
    }
}

pub(super) fn apply_profile(
    args: &mut cli::ProxyArgs,
    profile: &ProxyProfile,
    source: &str,
) -> Result<()> {
    let defaults = ssh_proxy_config::plan_profile_defaults(profile)?;

    if args.listen == SocketAddr::from(([127, 0, 0, 1], 1080))
        && let Some(value) = defaults.listen
    {
        args.listen = value;
    }
    args.tcp_target = args
        .tcp_target
        .take()
        .or_else(|| defaults.tcp_target.clone().map(Into::into));
    if args.ssh_args.is_empty() {
        args.ssh_args = defaults.ssh_args.clone();
    }
    args.user = args.user.take().or_else(|| defaults.user.clone());
    args.port = args.port.or(defaults.port);
    if args.identity.is_empty() {
        args.identity = defaults.identity.clone();
    }
    args.config = args.config.take().or_else(|| defaults.config.clone());
    args.known_hosts = args
        .known_hosts
        .take()
        .or_else(|| defaults.known_hosts.clone());
    if !args.accept_new {
        args.accept_new = defaults.accept_new.unwrap_or(false);
    }
    if !args.insecure_ignore_host_key {
        args.insecure_ignore_host_key = defaults.insecure_ignore_host_key.unwrap_or(false);
    }
    if args.jump.is_empty() {
        args.jump = defaults.jump.clone();
    }
    args.remote_path = args
        .remote_path
        .take()
        .or_else(|| defaults.remote_path.clone());
    args.remote_bin = args.remote_bin.take().or(defaults.remote_bin.clone());
    if let Some(value) = defaults.deployment {
        args.deploy = value.into();
    }
    if let Some(value) = defaults.remote_platform {
        args.remote_os = value.into();
    }
    if args.remote_transport == cli::RemoteTransport::Auto
        && let Some(transport) = defaults.transport
    {
        args.remote_transport = transport.into();
        if transport != ssh_proxy_core::model::TransportMode::Auto
            && args.transport_selection_source.is_none()
        {
            args.transport_selection_source = Some(source.to_string());
            args.transport_selection_reason =
                Some(format!("loaded from {source} remote_transport"));
        }
    }
    if let Some(value) = defaults.endpoint.remote_tcp {
        args.remote_tcp = value;
    }
    if let Some(value) = defaults.endpoint.remote_control {
        args.remote_control = value;
    }
    if args.remote_quic.is_none()
        && let Some(value) = defaults.endpoint.remote_quic
    {
        args.remote_quic = Some(value);
    }
    if !args.allow_plain_tcp {
        args.allow_plain_tcp = defaults.allow_plain_tcp.unwrap_or(false);
    }
    if args.remote_tls.is_none()
        && let Some(value) = defaults.endpoint.remote_tls
    {
        args.remote_tls = Some(value);
    }
    args.remote_ca = args
        .remote_ca
        .take()
        .or(defaults.endpoint.remote_ca.clone());
    if args.remote_name == "localhost"
        && let Some(value) = &defaults.endpoint.remote_name
    {
        args.remote_name = value.clone();
    }
    args.remote_client_cert = args
        .remote_client_cert
        .take()
        .or_else(|| defaults.endpoint.remote_client_cert.clone());
    args.remote_client_key = args
        .remote_client_key
        .take()
        .or_else(|| defaults.endpoint.remote_client_key.clone());
    args.remote_token = args
        .remote_token
        .take()
        .or_else(|| defaults.remote_token.clone());
    args.egress_proxy = args
        .egress_proxy
        .take()
        .or_else(|| defaults.endpoint.egress_proxy.clone());
    if let Some(value) = defaults.runtime.reconnect_delay_secs {
        args.reconnect_delay_secs = value;
    }
    if let Some(value) = defaults.runtime.reconnect_max_delay_secs {
        args.reconnect_max_delay_secs = value;
    }
    if let Some(value) = defaults.runtime.connect_timeout_secs {
        args.connect_timeout_secs = value;
    }
    if let Some(value) = defaults.runtime.quic.max_bidi_streams {
        args.quic_max_bidi_streams = value;
    }
    if let Some(value) = defaults.runtime.quic.stream_receive_window {
        args.quic_stream_receive_window = value;
    }
    if let Some(value) = defaults.runtime.quic.receive_window {
        args.quic_receive_window = value;
    }
    if let Some(value) = defaults.runtime.quic.keep_alive_interval_secs {
        args.quic_keep_alive_interval_secs = value;
    }
    if let Some(value) = defaults.runtime.quic.idle_timeout_secs {
        args.quic_idle_timeout_secs = value;
    }
    if let Some(value) = defaults.transport_pool_size {
        let effective = value.max(1);
        args.transport_pool_size = effective;
        args.transport_pool_source = Some(source.to_string());
        args.transport_pool_reason = Some(if value == effective {
            format!("loaded from {source} transport_pool_size")
        } else {
            format!("loaded from {source} transport_pool_size; clamped to minimum 1")
        });
    }
    if let Some(value) = defaults.runtime.workload_hint {
        args.workload_hint = Some(value.into());
    }
    if let Some(value) = defaults.ssh_session_pool_size
        && (args.ssh_session_pool_size.is_none()
            || (source == "profile" && args.ssh_session_pool_source.as_deref() == Some("defaults")))
    {
        let requested = value.max(1);
        let effective = if source == "defaults" {
            requested.min(2)
        } else {
            requested
        };
        args.ssh_session_pool_size = Some(effective);
        args.ssh_session_pool_source = Some(source.to_string());
        args.ssh_session_pool_reason = Some(if source == "defaults" && requested > effective {
            format!(
                "loaded from defaults ssh_session_pool_size={value}; capped to pool=2 because only command-line/profile benchmark experiments may exceed the implicit-safe ssh-native range"
            )
        } else if value == effective {
            format!("loaded from {source} ssh_session_pool_size")
        } else {
            format!("loaded from {source} ssh_session_pool_size; clamped to minimum 1")
        });
        args.ssh_session_pool_warning = if source == "defaults" && requested > effective {
            Some(
                "ssh-native defaults above 2 are not auto-selected; use --ssh-session-pool-size or a target profile for explicit benchmark experiments"
                    .to_string(),
            )
        } else {
            ssh_session_pool_warning(effective)
        };
    }
    if let Some(size) = args.ssh_session_pool_size
        && args.ssh_session_pool_source.is_none()
    {
        args.ssh_session_pool_source = Some("command-line".to_string());
        args.ssh_session_pool_reason = Some("loaded from --ssh-session-pool-size".to_string());
        args.ssh_session_pool_warning = ssh_session_pool_warning(size);
    }
    if !args.no_reconnect {
        args.no_reconnect = defaults.no_reconnect.unwrap_or(false);
    }
    args.control_listen = args.control_listen.or(defaults.endpoint.control_listen);
    Ok(())
}

pub(super) fn sorted_profiles(config: &AppConfig) -> Vec<(&String, &ProxyProfile)> {
    let mut profiles = config.profiles.iter().collect::<Vec<_>>();
    profiles.sort_by(|(left, _), (right, _)| left.cmp(right));
    profiles
}

pub(super) fn apply_profile_set(
    config: &mut AppConfig,
    args: cli::ConfigProfileSetArgs,
) -> Result<()> {
    if args.accept_new && args.no_accept_new {
        bail!("--accept-new and --no-accept-new are mutually exclusive");
    }
    if args.allow_plain_tcp && args.no_allow_plain_tcp {
        bail!("--allow-plain-tcp and --no-allow-plain-tcp are mutually exclusive");
    }
    if let Some(value) = &args.remote_transport {
        ssh_proxy_config::parse_transport_mode(value)?;
    }
    let profile = config.profiles.entry(args.name).or_default();
    set_opt(&mut profile.target, args.target);
    set_opt(&mut profile.user, args.user);
    set_opt(&mut profile.port, args.port);
    if !args.identity.is_empty() {
        profile.identity = args.identity;
    }
    set_opt(&mut profile.config, args.ssh_config);
    set_opt(&mut profile.known_hosts, args.known_hosts);
    if args.accept_new {
        profile.accept_new = Some(true);
    }
    if args.no_accept_new {
        profile.accept_new = Some(false);
    }
    if args.insecure_ignore_host_key {
        profile.insecure_ignore_host_key = Some(true);
    }
    if args.no_insecure_ignore_host_key {
        profile.insecure_ignore_host_key = Some(false);
    }
    if !args.jump.is_empty() {
        profile.jump = args.jump;
    }
    set_opt(&mut profile.listen, args.listen);
    set_opt(&mut profile.tcp_target, args.tcp_target.map(Into::into));
    set_opt(&mut profile.remote_transport, args.remote_transport);
    set_opt(&mut profile.remote_tcp, args.remote_tcp);
    set_opt(&mut profile.remote_control, args.remote_control);
    set_opt(&mut profile.remote_quic, args.remote_quic);
    set_opt(&mut profile.remote_tls, args.remote_tls);
    set_opt(
        &mut profile.quic_max_bidi_streams,
        args.quic_max_bidi_streams,
    );
    set_opt(
        &mut profile.quic_stream_receive_window,
        args.quic_stream_receive_window,
    );
    set_opt(&mut profile.quic_receive_window, args.quic_receive_window);
    set_opt(
        &mut profile.quic_keep_alive_interval_secs,
        args.quic_keep_alive_interval_secs,
    );
    set_opt(
        &mut profile.quic_idle_timeout_secs,
        args.quic_idle_timeout_secs,
    );
    set_opt(&mut profile.remote_ca, args.remote_ca);
    set_opt(&mut profile.remote_name, args.remote_name);
    set_opt(&mut profile.remote_client_cert, args.remote_client_cert);
    set_opt(&mut profile.remote_client_key, args.remote_client_key);
    set_opt(&mut profile.remote_token, args.remote_token);
    set_opt(&mut profile.egress_proxy, args.egress_proxy);
    if args.allow_plain_tcp {
        profile.allow_plain_tcp = Some(true);
    }
    if args.no_allow_plain_tcp {
        profile.allow_plain_tcp = Some(false);
    }
    set_opt(
        &mut profile.transport_pool_size,
        args.transport_pool_size.map(|value| value.max(1)),
    );
    set_opt(
        &mut profile.workload_hint,
        args.workload_hint.map(Into::into),
    );
    set_opt(
        &mut profile.ssh_session_pool_size,
        args.ssh_session_pool_size.map(|value| value.max(1)),
    );
    Ok(())
}

pub(super) fn set_opt<T>(slot: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *slot = value;
    }
}

fn ssh_session_pool_warning(size: usize) -> Option<String> {
    (size > 2).then(|| {
        "ssh-native session pools above 2 can lose to handshake and scheduling overhead; benchmark before relying on this explicit value"
            .to_string()
    })
}
