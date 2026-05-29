use super::*;
use clap::CommandFactory;
use ssh_proxy_core::{intent, model};

#[test]
fn production_help_hides_legacy_entrypoints() {
    let help = Cli::command().render_long_help().to_string();

    for visible in [
        "daemon", "up", "down", "status", "events", "doctor", "vscode",
    ] {
        assert!(
            help.contains(visible),
            "{visible} should stay visible in help"
        );
    }

    for hidden in [
        "proxy",
        "route",
        "reverse",
        "remote",
        "node",
        "install-remote",
        "config",
        "control",
        "host",
        "service",
    ] {
        assert!(
            !help.contains(&format!("  {hidden}")),
            "{hidden} should be hidden from production help"
        );
    }
}

#[test]
fn host_exec_accepts_stdin_json_shape() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "host",
        "edge",
        "exec",
        "--stdin",
        "--label",
        "remote setup",
        "--timeout-secs",
        "7",
        "--json",
    ])
    .unwrap();

    match cli.command {
        Commands::Host(args) => match args.command {
            HostCommand::Exec(exec) => {
                assert_eq!(args.target, "edge");
                assert!(exec.stdin);
                assert_eq!(exec.label, "remote setup");
                assert_eq!(exec.timeout_secs, 7);
                assert!(exec.json);
            }
            other => panic!("unexpected host command: {other:?}"),
        },
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn route_accepts_json_flag() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "route",
        "edge",
        "--direction",
        "remote-uses-local",
        "--json",
    ])
    .unwrap();

    match cli.command {
        Commands::Route(args) => {
            assert_eq!(args.target, "edge");
            assert_eq!(args.direction, RouteDirection::RemoteUsesLocal);
            assert!(args.json);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn service_accepts_json_flag() {
    let cli = Cli::try_parse_from(["ssh_proxy", "service", "--json", "status"]).unwrap();

    match cli.command {
        Commands::Service(args) => {
            assert!(args.json);
            assert!(matches!(args.command, ServiceCommand::Status));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn service_accepts_ensure_and_elevate() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "service",
        "--scope",
        "system",
        "--json",
        "--elevate",
        "ensure",
    ])
    .unwrap();

    match cli.command {
        Commands::Service(args) => {
            assert_eq!(args.scope, ServiceScope::System);
            assert!(args.json);
            assert!(args.elevate);
            assert!(matches!(args.command, ServiceCommand::Ensure));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn node_control_accepts_json_flag() {
    let cli = Cli::try_parse_from(["ssh_proxy", "node", "control", "--json", "status"]).unwrap();

    match cli.command {
        Commands::Node(args) => match args.command {
            NodeCommand::Control(control) => {
                assert!(control.json);
                assert!(matches!(control.command, NodeControlCommand::Status));
            }
            other => panic!("unexpected node command: {other:?}"),
        },
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn daemon_status_uses_v3_daemon_command() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "daemon",
        "--scope",
        "system",
        "--json",
        "status",
    ])
    .unwrap();

    match cli.command {
        Commands::Daemon(args) => {
            assert_eq!(args.scope, DaemonScope::System);
            assert!(args.json);
            assert!(matches!(args.command, DaemonCommand::Status));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn daemon_accepts_global_flags_after_subcommand() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "daemon",
        "install",
        "--scope",
        "system",
        "--elevate",
        "--json",
    ])
    .unwrap();

    match cli.command {
        Commands::Daemon(args) => {
            assert_eq!(args.scope, DaemonScope::System);
            assert!(args.json);
            match args.command {
                DaemonCommand::Install { elevate, no_copy } => {
                    assert!(elevate);
                    assert!(!no_copy);
                }
                other => panic!("unexpected daemon command: {other:?}"),
            }
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn up_accepts_proxy_session_shape() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "up",
        "--target",
        "126",
        "--workspace",
        "window-a",
        "--local-proxy",
        "http://127.0.0.1:10808/",
        "--json",
    ])
    .unwrap();

    match cli.command {
        Commands::Up(args) => {
            assert_eq!(args.target, "126");
            assert_eq!(args.workspace.as_deref(), Some("window-a"));
            assert_eq!(args.local_proxy, "http://127.0.0.1:10808/");
            assert_eq!(args.connect_mode, RouteConnectMode::Auto);
            assert!(args.json);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn vscode_up_accepts_workspace_session_shape() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "vscode",
        "up",
        "--target",
        "126",
        "--workspace",
        "window-a",
        "--local-proxy",
        "http://127.0.0.1:10808/",
        "--json",
    ])
    .unwrap();

    match cli.command {
        Commands::Vscode(args) => match args.command {
            VscodeCommand::Up(up) => {
                assert_eq!(up.target, "126");
                assert_eq!(up.workspace, "window-a");
                assert_eq!(up.local_proxy, "http://127.0.0.1:10808/");
                assert_eq!(up.connect_mode, RouteConnectMode::Auto);
                assert!(up.json);
            }
            other => panic!("unexpected vscode command: {other:?}"),
        },
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn cli_values_convert_to_command_neutral_models() {
    let target: model::TcpTarget = "example.com:443".parse::<TcpTarget>().unwrap().into();
    assert_eq!(target.host, "example.com");
    assert_eq!(target.port, 443);

    assert_eq!(
        model::TransportMode::from(RemoteTransport::TlsTcp),
        model::TransportMode::TlsTcp
    );
    assert_eq!(
        RemoteTransport::from(model::TransportMode::QuicNative),
        RemoteTransport::QuicNative
    );
    assert_eq!(
        model::RemotePlatform::from(RemoteOs::Windows),
        model::RemotePlatform::Windows
    );
    assert_eq!(
        PersistMode::from(model::PersistenceMode::Launchd),
        PersistMode::Launchd
    );
    assert_eq!(
        model::RouteDirection::from(RouteDirection::RemoteUsesLocal),
        model::RouteDirection::RemoteUsesLocal
    );
    assert_eq!(
        model::RouteConnectMode::from(RouteConnectMode::ReverseLink),
        model::RouteConnectMode::ReverseLink
    );
    assert_eq!(
        model::WorkloadHint::from(RouteWorkloadHint::Concurrent),
        model::WorkloadHint::Concurrent
    );
    assert_eq!(
        intent::DeploymentPolicy::from(DeployMode::Always),
        intent::DeploymentPolicy::Always
    );
}

#[test]
fn proxy_args_convert_to_route_intent_without_cli_types() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "proxy",
        "edge",
        "--listen",
        "127.0.0.1:1082",
        "--tcp-target",
        "db.internal:5432",
        "--remote-transport",
        "tls-tcp",
        "--remote-tls",
        "127.0.0.1:19443",
        "--remote-ca",
        "ca.pem",
        "--control-listen",
        "127.0.0.1:1083",
        "--transport-pool-size",
        "4",
        "--no-reconnect",
    ])
    .unwrap();

    match cli.command {
        Commands::Proxy(args) => {
            let intent = intent::RouteIntent::from(&args);

            assert_eq!(intent.ssh.target, "edge");
            assert_eq!(intent.direction, model::RouteDirection::LocalUsesRemote);
            assert_eq!(intent.transport, model::TransportMode::TlsTcp);
            assert_eq!(
                intent.endpoint.listen.unwrap().to_string(),
                "127.0.0.1:1082"
            );
            assert_eq!(
                intent.endpoint.control_listen.unwrap().to_string(),
                "127.0.0.1:1083"
            );
            assert_eq!(
                intent.endpoint.remote_tls.unwrap().to_string(),
                "127.0.0.1:19443"
            );
            assert_eq!(intent.endpoint.tcp_target.unwrap().host, "db.internal");
            assert_eq!(intent.runtime.transport_pool_size, Some(4));
            assert!(intent.runtime.no_reconnect);
            assert!(!intent.persist);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn route_args_convert_to_route_intent_with_runtime_policy() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "route",
        "edge",
        "--direction",
        "remote-uses-local",
        "--connect-mode",
        "direct",
        "--bind",
        "0.0.0.0",
        "--port",
        "18080",
        "--remote-transport",
        "quic-native",
        "--local-peer",
        "192.0.2.10:19080",
        "--volatile",
    ])
    .unwrap();

    match cli.command {
        Commands::Route(args) => {
            let intent = intent::RouteIntent::from(&args);

            assert_eq!(intent.direction, model::RouteDirection::RemoteUsesLocal);
            assert_eq!(intent.connect_mode, model::RouteConnectMode::Direct);
            assert_eq!(intent.transport, model::TransportMode::QuicNative);
            assert_eq!(intent.endpoint.listen.unwrap().to_string(), "0.0.0.0:18080");
            assert_eq!(
                intent.endpoint.local_peer.unwrap().to_string(),
                "192.0.2.10:19080"
            );
            assert!(!intent.persist);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn install_args_convert_to_remote_install_intent() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "install-remote",
        "edge",
        "--remote-os",
        "unix",
        "--remote-tcp",
        "127.0.0.1:29080",
        "--remote-control",
        "127.0.0.1:29081",
        "--remote-tls-transport",
        "127.0.0.1:29443",
        "--persist",
        "systemd",
        "--local-node-id",
        "local-node",
    ])
    .unwrap();

    match cli.command {
        Commands::InstallRemote(args) => {
            let intent = intent::RemoteInstallIntent::from(&args);

            assert_eq!(intent.ssh.target, "edge");
            assert_eq!(intent.remote_platform, model::RemotePlatform::Unix);
            assert_eq!(intent.persistence, model::PersistenceMode::Systemd);
            assert_eq!(intent.remote_tcp.to_string(), "127.0.0.1:29080");
            assert_eq!(intent.remote_control.to_string(), "127.0.0.1:29081");
            assert_eq!(
                intent.remote_tls_transport.unwrap().to_string(),
                "127.0.0.1:29443"
            );
            assert_eq!(intent.local_node_id.as_deref(), Some("local-node"));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn peer_bootstrap_args_convert_to_bootstrap_intent() {
    let cli = Cli::try_parse_from([
        "ssh_proxy",
        "node",
        "control",
        "peer-bootstrap",
        "edge",
        "--alias",
        "prod",
        "--force",
        "--remote-token",
        "secret",
    ])
    .unwrap();

    match cli.command {
        Commands::Node(args) => match args.command {
            NodeCommand::Control(control) => match control.command {
                NodeControlCommand::PeerBootstrap(args) => {
                    let intent = intent::PeerBootstrapIntent::from(&args);

                    assert_eq!(intent.install.ssh.target, "edge");
                    assert_eq!(intent.alias.as_deref(), Some("prod"));
                    assert!(intent.force);
                    assert_eq!(intent.install.remote_token.as_deref(), Some("secret"));
                    assert_eq!(intent.install.persistence, model::PersistenceMode::None);
                }
                other => panic!("unexpected control command: {other:?}"),
            },
            other => panic!("unexpected node command: {other:?}"),
        },
        other => panic!("unexpected command: {other:?}"),
    }
}
