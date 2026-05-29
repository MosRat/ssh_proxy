use super::*;
use clap::CommandFactory;

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
