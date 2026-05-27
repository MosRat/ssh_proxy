use super::*;

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
