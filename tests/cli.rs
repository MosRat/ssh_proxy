use std::process::Command;

#[test]
fn config_sample_prints_valid_toml() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["config", "sample"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("sample config should be utf-8");
    let value: toml::Value = toml::from_str(&stdout).expect("sample config should be valid TOML");
    assert_eq!(value["defaults"]["listen"].as_str(), Some("127.0.0.1:1080"));
    assert_eq!(value["defaults"]["remote_transport"].as_str(), Some("auto"));
    assert_eq!(value["identity"]["node_id"].as_str(), Some("spx-generated"));
    assert_eq!(
        value["peers"]["office"]["trust"].as_str(),
        Some("ssh-bootstrap")
    );
    assert_eq!(
        value["peers"]["office"]["token_metadata"]["scope"].as_str(),
        Some("peer-control-transport")
    );
    assert_eq!(
        value["profiles"]["office"]["jump"][0].as_str(),
        Some("bastion.example.com")
    );
}

#[test]
fn route_dry_run_submits_daemon_intent() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args([
            "route",
            "office",
            "--direction",
            "remote-uses-local",
            "--port",
            "18080",
            "--tcp-target",
            "example.com:443",
            "--dry-run",
        ])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("dry run should be utf-8");
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("route dry-run should print JSON");
    assert_eq!(value["cmd"].as_str(), Some("route_intent"));
    assert_eq!(
        value["route"]["direction"].as_str(),
        Some("remote-uses-local")
    );
    assert_eq!(value["route"]["port"].as_u64(), Some(18080));
    assert_eq!(
        value["route"]["tcp_target"]["host"].as_str(),
        Some("example.com")
    );
    assert_eq!(value["route"]["tcp_target"]["port"].as_u64(), Some(443));
}

#[test]
fn route_explain_prints_expanded_plan() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args([
            "route",
            "office",
            "--direction",
            "local-uses-remote",
            "--port",
            "18080",
            "--remote-tls",
            "192.0.2.1:19082",
            "--explain",
        ])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("explain should be utf-8");
    let value: serde_json::Value =
        serde_json::from_str(&stdout).expect("route explain should print JSON");
    assert_eq!(value["direction"].as_str(), Some("local-uses-remote"));
    assert_eq!(value["mode"].as_str(), Some("local-forward"));
    assert_eq!(value["topology"].is_object(), true);
    assert_eq!(value["preflight"].is_object(), true);
    assert_eq!(value["decision_chain"].is_object(), true);
    assert_eq!(
        value["decision_chain"]["topology"]["class"].as_str(),
        Some("ssh-only")
    );
    assert_eq!(
        value["decision_chain"]["preflight"]["repair_hint"].as_str(),
        Some("use ssh-native fallback, or publish a peer endpoint reachable from this client")
    );
    assert_eq!(
        value["decision_chain"]["selected_transport"].as_str(),
        Some("ssh-native")
    );
}

#[test]
fn cli_exposes_daemon_host_and_service_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .arg("--help")
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help should be utf-8");
    assert!(help.contains("daemon"));
    assert!(help.contains("node"));
    assert!(help.contains("reverse"));
    assert!(help.contains("host"));
    assert!(help.contains("service"));
}

#[test]
fn service_print_shows_daemon_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["service", "print"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("service output should be utf-8");
    assert!(text.contains("daemon serve --control"));
}

#[test]
fn service_help_exposes_stable_install_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["service", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("service help should be utf-8");
    assert!(help.contains("install-dir"));
    assert!(help.contains("no-copy"));
    assert!(help.contains("transport"));
    assert!(help.contains("token"));
}

#[test]
fn config_help_exposes_inspect() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["config", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("config help should be utf-8");
    assert!(help.contains("inspect"));
    assert!(help.contains("export-descriptor"));
    assert!(help.contains("import-descriptor"));
}

#[test]
fn host_help_exposes_management_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["host", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help should be utf-8");
    assert!(help.contains("logs"));
    assert!(help.contains("doctor"));
    assert!(help.contains("clean"));
}

#[test]
fn reverse_help_exposes_remote_listener() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["reverse", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help should be utf-8");
    assert!(help.contains("remote-listen"));
}

#[test]
fn proxy_help_exposes_auto_transport() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["proxy", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("proxy help should be utf-8");
    assert!(help.contains("auto"));
    assert!(help.contains("quic"));
    assert!(help.contains("remote-quic"));
    assert!(help.contains("tls-tcp"));
    assert!(help.contains("plain-tcp"));
    assert!(help.contains("allow-plain-tcp"));
    assert!(help.contains("remote-tls"));
    assert!(help.contains("remote-ca"));
    assert!(help.contains("remote-client-cert"));
    assert!(help.contains("remote-client-key"));
    assert!(help.contains("remote-transport"));
    assert!(help.contains("tcp-target"));
}

#[test]
fn remote_help_exposes_reverse_socks() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["remote", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help should be utf-8");
    assert!(help.contains("reverse-socks"));
}

#[test]
fn node_help_exposes_daemon_and_control() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["node", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("node help should be utf-8");
    assert!(help.contains("daemon"));
    assert!(help.contains("control"));
}

#[test]
fn node_control_help_exposes_symmetric_routes() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["node", "control", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("node control help should be utf-8");
    assert!(help.contains("forward"));
    assert!(help.contains("reverse"));
    assert!(help.contains("route-plan"));
    assert!(help.contains("stop-route"));
    assert!(help.contains("restart-route"));
    assert!(help.contains("routes"));
    assert!(help.contains("peers"));
    assert!(help.contains("nodes"));
    assert!(help.contains("jobs"));
    assert!(help.contains("node-ensure"));
    assert!(help.contains("node-start"));
    assert!(help.contains("node-stop"));
    assert!(help.contains("node-restart"));
    assert!(help.contains("token-rotate"));
    assert!(help.contains("peer-bootstrap"));
    assert!(help.contains("peer-ensure"));
    assert!(help.contains("peer-update"));
    assert!(help.contains("peer-refresh"));
    assert!(help.contains("peer-diff"));
    assert!(help.contains("peer-reconcile"));
    assert!(help.contains("peer-check-version"));
    assert!(help.contains("peer-rotate-token"));
    assert!(help.contains("peer-forget"));
}

#[test]
fn node_daemon_help_exposes_tls_transport() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["node", "daemon", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("node daemon help should be utf-8");
    assert!(help.contains("tls-transport"));
    assert!(help.contains("quic-transport"));
    assert!(help.contains("tls-cert"));
    assert!(help.contains("tls-key"));
    assert!(help.contains("tls-client-ca"));
}

#[test]
fn host_help_exposes_node_management_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"))
        .args(["host", "--help"])
        .output()
        .expect("failed to run ssh_proxy");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("host help should be utf-8");
    assert!(help.contains("node-descriptor"));
    assert!(help.contains("node-status"));
    assert!(help.contains("node-forward"));
    assert!(help.contains("node-reverse"));
    assert!(help.contains("node-stop-route"));
    assert!(help.contains("node-restart-route"));
    assert!(help.contains("node-routes"));
    assert!(help.contains("node-connect"));
}
