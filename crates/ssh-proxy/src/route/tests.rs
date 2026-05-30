use super::*;
use crate::{cli, config, node_daemon};

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
    let plan =
        remote_uses_local_direct_plan(&args, forward.id.as_deref().unwrap(), forward, local_peer);

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
    let mut forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
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
    let mut forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
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
        Some(
            "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
                .to_string(),
        ),
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
    let mut forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
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
    let mut forward = node_forward_from_route(&args, &config, "peer".to_string(), false).unwrap();
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
