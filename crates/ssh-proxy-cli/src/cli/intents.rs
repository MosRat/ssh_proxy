use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use ssh_proxy_core::{
    intent::{
        PeerBootstrapIntent, QuicRuntimeTuningIntent, RemoteInstallIntent, RouteEndpointIntent,
        RouteIntent, RuntimeTuningIntent, SshTargetIntent,
    },
    model::{self, PersistenceMode, RouteConnectMode, RouteDirection, TransportMode},
};

use super::{
    InstallRemoteArgs, NodeForwardArgs, NodeReverseArgs, PeerBootstrapArgs, ProxyArgs, ReverseArgs,
    ReverseTaskArgs, RouteArgs, TcpTarget,
};

impl From<&ProxyArgs> for RouteIntent {
    fn from(args: &ProxyArgs) -> Self {
        let mut intent = RouteIntent::new(proxy_ssh_target(args), RouteDirection::LocalUsesRemote);
        intent.transport = args.remote_transport.into();
        intent.remote_platform = args.remote_os.into();
        intent.deployment = args.deploy.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.remote_token = args.remote_token.clone();
        intent.endpoint = proxy_endpoint(args);
        intent.runtime = runtime_tuning(
            Some(args.reconnect_delay_secs),
            Some(args.reconnect_max_delay_secs),
            Some(args.connect_timeout_secs),
            Some(args.transport_pool_size),
            args.ssh_session_pool_size,
            args.workload_hint.map(Into::into),
            quic_tuning(
                Some(args.quic_max_bidi_streams),
                Some(args.quic_stream_receive_window),
                Some(args.quic_receive_window),
                Some(args.quic_keep_alive_interval_secs),
                Some(args.quic_idle_timeout_secs),
            ),
            args.no_reconnect,
        );
        intent.persist = false;
        intent
    }
}

impl From<&RouteArgs> for RouteIntent {
    fn from(args: &RouteArgs) -> Self {
        let mut intent = RouteIntent::new(
            route_ssh_target(args),
            model::RouteDirection::from(args.direction),
        );
        intent.connect_mode = args.connect_mode.into();
        intent.transport = args.remote_transport.into();
        intent.remote_platform = args.remote_os.into();
        intent.deployment = args.deploy.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.remote_token = args.remote_token.clone();
        intent.endpoint = route_endpoint(args);
        intent.runtime = runtime_tuning(
            args.reconnect_delay_secs,
            args.reconnect_max_delay_secs,
            args.connect_timeout_secs,
            args.transport_pool_size,
            args.ssh_session_pool_size,
            args.workload_hint.map(Into::into),
            quic_tuning(
                args.quic_max_bidi_streams,
                args.quic_stream_receive_window,
                args.quic_receive_window,
                args.quic_keep_alive_interval_secs,
                args.quic_idle_timeout_secs,
            ),
            args.no_reconnect,
        );
        intent.id = args.id.clone();
        intent.persist = !args.volatile;
        intent
    }
}

impl From<&NodeForwardArgs> for RouteIntent {
    fn from(args: &NodeForwardArgs) -> Self {
        let mut intent = RouteIntent::new(
            node_forward_ssh_target(args),
            RouteDirection::LocalUsesRemote,
        );
        intent.transport = args.remote_transport.into();
        intent.remote_platform = args.remote_os.into();
        intent.deployment = args.deploy.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.remote_token = args.remote_token.clone();
        intent.endpoint = node_forward_endpoint(args);
        intent.runtime = runtime_tuning(
            Some(args.reconnect_delay_secs),
            Some(args.reconnect_max_delay_secs),
            Some(args.connect_timeout_secs),
            Some(args.transport_pool_size),
            args.ssh_session_pool_size,
            args.workload_hint.map(Into::into),
            quic_tuning(
                Some(args.quic_max_bidi_streams),
                Some(args.quic_stream_receive_window),
                Some(args.quic_receive_window),
                Some(args.quic_keep_alive_interval_secs),
                Some(args.quic_idle_timeout_secs),
            ),
            args.no_reconnect,
        );
        intent.id = args.id.clone();
        intent.persist = !args.volatile;
        intent
    }
}

impl From<&ReverseArgs> for RouteIntent {
    fn from(args: &ReverseArgs) -> Self {
        let mut intent =
            RouteIntent::new(reverse_ssh_target(args), RouteDirection::RemoteUsesLocal);
        intent.connect_mode = RouteConnectMode::ReverseLink;
        intent.transport = TransportMode::Exec;
        intent.remote_platform = args.remote_os.into();
        intent.deployment = args.deploy.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.endpoint =
            reverse_endpoint(args.remote_listen, &args.tcp_target, &args.egress_proxy);
        intent.runtime = runtime_tuning(
            Some(args.reconnect_delay_secs),
            Some(args.reconnect_max_delay_secs),
            Some(args.connect_timeout_secs),
            Some(args.transport_pool_size),
            None,
            None,
            QuicRuntimeTuningIntent::default(),
            args.no_reconnect,
        );
        intent.persist = false;
        intent
    }
}

impl From<&ReverseTaskArgs> for RouteIntent {
    fn from(args: &ReverseTaskArgs) -> Self {
        let mut intent = RouteIntent::new(
            reverse_task_ssh_target(args),
            RouteDirection::RemoteUsesLocal,
        );
        intent.connect_mode = RouteConnectMode::ReverseLink;
        intent.transport = TransportMode::Exec;
        intent.remote_platform = args.remote_os.into();
        intent.deployment = args.deploy.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.endpoint =
            reverse_endpoint(args.remote_listen, &args.tcp_target, &args.egress_proxy);
        intent.runtime = runtime_tuning(
            Some(args.reconnect_delay_secs),
            Some(args.reconnect_max_delay_secs),
            Some(args.connect_timeout_secs),
            None,
            None,
            None,
            QuicRuntimeTuningIntent::default(),
            args.no_reconnect,
        );
        intent.persist = true;
        intent
    }
}

impl From<&NodeReverseArgs> for RouteIntent {
    fn from(args: &NodeReverseArgs) -> Self {
        let mut intent = RouteIntent::new(
            node_reverse_ssh_target(args),
            RouteDirection::RemoteUsesLocal,
        );
        intent.connect_mode = RouteConnectMode::ReverseLink;
        intent.transport = TransportMode::Exec;
        intent.remote_platform = args.remote_os.into();
        intent.deployment = args.deploy.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.endpoint =
            reverse_endpoint(args.remote_listen, &args.tcp_target, &args.egress_proxy);
        intent.runtime = runtime_tuning(
            Some(args.reconnect_delay_secs),
            Some(args.reconnect_max_delay_secs),
            Some(args.connect_timeout_secs),
            None,
            None,
            None,
            QuicRuntimeTuningIntent::default(),
            args.no_reconnect,
        );
        intent.id = args.id.clone();
        intent.persist = !args.volatile;
        intent
    }
}

impl From<&InstallRemoteArgs> for RemoteInstallIntent {
    fn from(args: &InstallRemoteArgs) -> Self {
        let mut intent = RemoteInstallIntent::new(
            install_ssh_target(args),
            args.remote_tcp,
            args.remote_control,
            args.persist.into(),
        );
        intent.remote_platform = args.remote_os.into();
        intent.remote_path = args.remote_path.clone();
        intent.remote_bin = args.remote_bin.clone();
        intent.remote_token = args.remote_token.clone();
        intent.local_node_id = args.local_node_id.clone();
        intent.local_node_name = args.local_node_name.clone();
        intent.local_control_endpoint = args.local_control_endpoint.clone();
        intent.local_transport = args.local_transport;
        intent.remote_node_id = args.remote_node_id.clone();
        intent.remote_node_name = args.remote_node_name.clone();
        intent.remote_tls_transport = args.remote_tls_transport;
        intent.remote_quic_transport = args.remote_quic_transport;
        intent.remote_tls_cert = args.remote_tls_cert.clone();
        intent.remote_tls_key = args.remote_tls_key.clone();
        intent.remote_tls_client_ca = args.remote_tls_client_ca.clone();
        intent
    }
}

impl From<&PeerBootstrapArgs> for PeerBootstrapIntent {
    fn from(args: &PeerBootstrapArgs) -> Self {
        let mut install = RemoteInstallIntent::new(
            peer_bootstrap_ssh_target(args),
            args.remote_tcp,
            args.remote_control,
            PersistenceMode::None,
        );
        install.remote_platform = args.remote_os.into();
        install.remote_path = args.remote_path.clone();
        install.remote_bin = args.remote_bin.clone();
        install.remote_token = args.remote_token.clone();
        let mut intent = PeerBootstrapIntent::new(install);
        intent.alias = args.alias.clone();
        intent.force = args.force;
        intent
    }
}

fn ssh_target(
    target: &str,
    ssh_args: &[String],
    ssh_command: Option<&String>,
    user: &Option<String>,
    port: Option<u16>,
    identity: &[PathBuf],
    config: &Option<PathBuf>,
    known_hosts: &Option<PathBuf>,
    accept_new: bool,
    insecure_ignore_host_key: bool,
    jump: &[String],
) -> SshTargetIntent {
    let mut intent = SshTargetIntent::new(target.to_string());
    intent.ssh_args = ssh_args.to_vec();
    intent.ssh_command = ssh_command.cloned();
    intent.user = user.clone();
    intent.port = port;
    intent.identity = identity.to_vec();
    intent.config = config.clone();
    intent.known_hosts = known_hosts.clone();
    intent.accept_new = accept_new;
    intent.insecure_ignore_host_key = insecure_ignore_host_key;
    intent.jump = jump.to_vec();
    intent
}

fn proxy_ssh_target(args: &ProxyArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        args.ssh_command.as_ref(),
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn route_ssh_target(args: &RouteArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        None,
        &args.user,
        args.ssh_port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn node_forward_ssh_target(args: &NodeForwardArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        None,
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn reverse_ssh_target(args: &ReverseArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        None,
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn reverse_task_ssh_target(args: &ReverseTaskArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        None,
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn node_reverse_ssh_target(args: &NodeReverseArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        None,
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn install_ssh_target(args: &InstallRemoteArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        args.ssh_command.as_ref(),
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn peer_bootstrap_ssh_target(args: &PeerBootstrapArgs) -> SshTargetIntent {
    ssh_target(
        &args.target,
        &args.ssh_args,
        None,
        &args.user,
        args.port,
        &args.identity,
        &args.config,
        &args.known_hosts,
        args.accept_new,
        args.insecure_ignore_host_key,
        &args.jump,
    )
}

fn proxy_endpoint(args: &ProxyArgs) -> RouteEndpointIntent {
    RouteEndpointIntent {
        listen: Some(args.listen),
        control_listen: args.control_listen,
        tcp_target: core_tcp_target(&args.tcp_target),
        remote_tcp: Some(args.remote_tcp),
        remote_control: Some(args.remote_control),
        remote_quic: args.remote_quic,
        remote_tls: args.remote_tls,
        remote_name: Some(args.remote_name.clone()),
        remote_ca: clone_path(args.remote_ca.as_deref()),
        remote_client_cert: clone_path(args.remote_client_cert.as_deref()),
        remote_client_key: clone_path(args.remote_client_key.as_deref()),
        egress_proxy: args.egress_proxy.clone(),
        allow_plain_tcp: args.allow_plain_tcp,
        ..Default::default()
    }
}

fn route_endpoint(args: &RouteArgs) -> RouteEndpointIntent {
    RouteEndpointIntent {
        listen: Some(SocketAddr::new(args.bind, args.port)),
        local_peer: args.local_peer,
        tcp_target: core_tcp_target(&args.tcp_target),
        remote_tcp: args.remote_tcp,
        remote_control: args.remote_control,
        remote_quic: args.remote_quic,
        remote_tls: args.remote_tls,
        remote_name: Some(args.remote_name.clone()),
        remote_ca: clone_path(args.remote_ca.as_deref()),
        egress_proxy: args.egress_proxy.clone(),
        allow_plain_tcp: args.allow_plain_tcp,
        ..Default::default()
    }
}

fn node_forward_endpoint(args: &NodeForwardArgs) -> RouteEndpointIntent {
    RouteEndpointIntent {
        listen: Some(args.listen),
        tcp_target: core_tcp_target(&args.tcp_target),
        remote_tcp: Some(args.remote_tcp),
        remote_control: Some(args.remote_control),
        remote_quic: args.remote_quic,
        remote_tls: args.remote_tls,
        remote_name: Some(args.remote_name.clone()),
        remote_ca: clone_path(args.remote_ca.as_deref()),
        remote_client_cert: clone_path(args.remote_client_cert.as_deref()),
        remote_client_key: clone_path(args.remote_client_key.as_deref()),
        egress_proxy: args.egress_proxy.clone(),
        allow_plain_tcp: args.allow_plain_tcp,
        ..Default::default()
    }
}

fn reverse_endpoint(
    remote_listen: SocketAddr,
    tcp_target: &Option<TcpTarget>,
    egress_proxy: &Option<String>,
) -> RouteEndpointIntent {
    RouteEndpointIntent {
        remote_listen: Some(remote_listen),
        tcp_target: core_tcp_target(tcp_target),
        egress_proxy: egress_proxy.clone(),
        ..Default::default()
    }
}

fn core_tcp_target(value: &Option<TcpTarget>) -> Option<model::TcpTarget> {
    value.clone().map(Into::into)
}

fn clone_path(value: Option<&Path>) -> Option<PathBuf> {
    value.map(Path::to_path_buf)
}

fn runtime_tuning(
    reconnect_delay_secs: Option<u64>,
    reconnect_max_delay_secs: Option<u64>,
    connect_timeout_secs: Option<u64>,
    transport_pool_size: Option<usize>,
    ssh_session_pool_size: Option<usize>,
    workload_hint: Option<model::WorkloadHint>,
    quic: QuicRuntimeTuningIntent,
    no_reconnect: bool,
) -> RuntimeTuningIntent {
    RuntimeTuningIntent {
        reconnect_delay_secs,
        reconnect_max_delay_secs,
        connect_timeout_secs,
        transport_pool_size,
        ssh_session_pool_size,
        workload_hint,
        quic,
        no_reconnect,
    }
}

fn quic_tuning(
    max_bidi_streams: Option<u32>,
    stream_receive_window: Option<u32>,
    receive_window: Option<u32>,
    keep_alive_interval_secs: Option<u64>,
    idle_timeout_secs: Option<u64>,
) -> QuicRuntimeTuningIntent {
    QuicRuntimeTuningIntent {
        max_bidi_streams,
        stream_receive_window,
        receive_window,
        keep_alive_interval_secs,
        idle_timeout_secs,
    }
}
