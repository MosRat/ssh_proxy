mod support;

use std::{fs, net::TcpListener, process::Stdio, thread, time::Duration};

use support::node_daemon::*;

#[test]
fn node_daemon_returns_json_errors_and_preflights_route_ports() {
    let control = free_addr();
    let transport = free_addr();
    let endpoint = format!("tcp://{control}");
    let child = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            &endpoint,
            "--transport",
            &transport.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(ChildGuard::new)
        .expect("start node daemon");

    wait_for_status(&endpoint);

    let missing = run_control(
        &endpoint,
        &["send", r#"{"cmd":"route_stop","id":"missing-route"}"#],
    );
    assert!(missing.status.success());
    let missing_json: serde_json::Value =
        serde_json::from_slice(&missing.stdout).expect("json response");
    assert_eq!(missing_json["ok"], false);
    assert!(
        missing_json["error"]
            .as_str()
            .expect("error string")
            .contains("missing-route")
    );

    let occupied = TcpListener::bind(("127.0.0.1", 0)).expect("occupy listener");
    let occupied_addr = occupied.local_addr().expect("occupied addr");
    let conflict = run_control(
        &endpoint,
        &[
            "forward",
            "does-not-need-to-resolve",
            "--id",
            "conflict",
            "--listen",
            &occupied_addr.to_string(),
        ],
    );
    assert!(conflict.status.success());
    let conflict_json: serde_json::Value =
        serde_json::from_slice(&conflict.stdout).expect("json response");
    assert_eq!(conflict_json["ok"], false);
    assert!(
        conflict_json["error"]
            .as_str()
            .expect("error string")
            .contains("already in use")
    );

    drop(occupied);
    stop_child(child, &endpoint);
}

#[test]
fn node_daemon_reuses_duplicate_route_start_for_same_spec() {
    let control = free_addr();
    let transport = free_addr();
    let listen = free_addr();
    let other_listen = free_addr();
    let endpoint = format!("tcp://{control}");
    let routes_path = route_store_path("route-reuse");
    let child = start_daemon(&endpoint, transport, &routes_path);

    wait_for_status(&endpoint);

    let started = run_control(
        &endpoint,
        &[
            "forward",
            "127.0.0.1",
            "--id",
            "duplicate",
            "--listen",
            &listen.to_string(),
            "--deploy",
            "never",
        ],
    );
    assert!(started.status.success());
    let started_json: serde_json::Value =
        serde_json::from_slice(&started.stdout).expect("start json");
    assert_eq!(started_json["ok"], true);
    assert_eq!(started_json["reused_existing"], false);

    let reused = run_control(
        &endpoint,
        &[
            "forward",
            "127.0.0.1",
            "--id",
            "duplicate",
            "--listen",
            &listen.to_string(),
            "--deploy",
            "never",
        ],
    );
    assert!(reused.status.success());
    let reused_json: serde_json::Value =
        serde_json::from_slice(&reused.stdout).expect("reuse json");
    assert_eq!(reused_json["ok"], true);
    assert_eq!(reused_json["id"], "duplicate");
    assert_eq!(reused_json["reused_existing"], true);
    assert_eq!(reused_json["listen"], listen.to_string());

    let mismatch = run_control(
        &endpoint,
        &[
            "forward",
            "127.0.0.1",
            "--id",
            "duplicate",
            "--listen",
            &other_listen.to_string(),
            "--deploy",
            "never",
        ],
    );
    assert!(mismatch.status.success());
    let mismatch_json: serde_json::Value =
        serde_json::from_slice(&mismatch.stdout).expect("mismatch json");
    assert_eq!(mismatch_json["ok"], false);
    assert!(
        mismatch_json["error"]
            .as_str()
            .expect("error string")
            .contains("different spec")
    );

    stop_child(child, &endpoint);
    let _ = fs::remove_file(routes_path);
}

#[test]
fn node_tcp_control_requires_token_when_configured() {
    let control = free_addr();
    let transport = free_addr();
    let endpoint = format!("tcp://{control}");
    let routes_path = route_store_path("control-auth-routes");
    let home = temp_dir("control-auth-home");
    let token = "control-test-token";
    let child = start_daemon_with_token(&endpoint, transport, &routes_path, token, &home);
    wait_tcp(control);

    let rejected = raw_control_request(control, r#"{"api_version":1,"cmd":"status"}"#);
    let rejected_ok = rejected["ok"] == false && rejected["code"] == "unauthorized";

    let accepted = run_control_with_token(&endpoint, token, &["status"]);
    assert!(accepted.status.success());
    let accepted_json: serde_json::Value =
        serde_json::from_slice(&accepted.stdout).expect("status json");
    let accepted_ok = accepted_json["ok"] == true && accepted_json["api_version"] == 1;
    assert_eq!(accepted_json["auth"]["control_token"], true);
    assert_eq!(
        accepted_json["auth"]["token_metadata"]["scope"],
        "daemon-control-transport"
    );

    let descriptor = run_control_with_token(&endpoint, token, &["descriptor"]);
    assert!(descriptor.status.success());
    let descriptor_json: serde_json::Value =
        serde_json::from_slice(&descriptor.stdout).expect("descriptor json");
    assert_eq!(
        descriptor_json["auth"]["token_metadata"]["scope"],
        "daemon-control-transport"
    );

    let rotated = run_control_with_token(&endpoint, token, &["token-rotate"]);
    assert!(rotated.status.success());
    let rotated_json: serde_json::Value =
        serde_json::from_slice(&rotated.stdout).expect("token rotate json");
    assert_eq!(rotated_json["ok"], true);
    assert_eq!(
        rotated_json["token_metadata"]["scope"],
        "daemon-control-transport"
    );
    let new_token = rotated_json["token"].as_str().expect("new token");
    assert_ne!(new_token, token);

    let old_rejected = raw_control_request(
        control,
        &format!(r#"{{"api_version":1,"cmd":"status","auth_token":"{token}"}}"#),
    );
    assert_eq!(old_rejected["ok"], false);
    assert_eq!(old_rejected["code"], "unauthorized");

    let new_accepted = run_control_with_token(&endpoint, new_token, &["status"]);
    assert!(new_accepted.status.success());

    stop_child_with_token(child, &endpoint, new_token);
    let _ = fs::remove_file(routes_path);
    let _ = fs::remove_dir_all(home);

    assert!(rejected_ok, "{rejected}");
    assert!(accepted_ok, "{accepted_json}");
}

#[test]
fn node_tcp_control_rejects_oversized_requests() {
    let control = free_addr();
    let transport = free_addr();
    let endpoint = format!("tcp://{control}");
    let routes_path = route_store_path("oversized-control-routes");
    let home = temp_dir("oversized-control-home");
    let child = start_daemon_with_home(&endpoint, transport, &routes_path, &home);
    wait_tcp(control);

    let oversized = format!("{}\n", "x".repeat(1024 * 1024 + 1));
    let response = raw_control_request(control, &oversized);

    stop_child(child, &endpoint);
    let _ = fs::remove_file(routes_path);
    let _ = fs::remove_dir_all(home);

    assert_eq!(response["ok"], false);
    assert_eq!(response["code"], "bad_request");
    assert!(
        response["error"]
            .as_str()
            .expect("error")
            .contains("too large")
    );
}

#[test]
fn node_daemon_materializes_identity_in_memory_and_lists_peers() {
    let endpoint_addr = free_addr();
    let endpoint = format!("tcp://{endpoint_addr}");
    let transport = free_addr();
    let routes_path = route_store_path("identity-routes");
    let home = temp_dir("identity-home");
    let child = start_daemon_with_home(&endpoint, transport, &routes_path, &home);
    wait_for_status(&endpoint);

    let status = run_control(&endpoint, &["status"]);
    assert!(status.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("status json");
    assert_eq!(status_json["ok"], true);
    assert!(
        status_json["node_id"]
            .as_str()
            .expect("node id")
            .starts_with("spx-")
    );

    let descriptor = run_control(&endpoint, &["descriptor"]);
    assert!(descriptor.status.success());
    let descriptor_json: serde_json::Value =
        serde_json::from_slice(&descriptor.stdout).expect("descriptor json");
    assert_eq!(descriptor_json["ok"], true);
    assert_eq!(descriptor_json["kind"], "peer_descriptor");
    assert_eq!(descriptor_json["control_api_version"], 1);
    assert_eq!(descriptor_json["peer_protocol_version"], 1);
    assert!(descriptor_json["service_instance_id"].is_string());
    assert!(descriptor_json["feature_bits"]["frames-v1"].as_bool() == Some(true));
    assert!(descriptor_json["os_user"].is_string());
    assert!(descriptor_json["data_dir"].is_string());
    assert_eq!(
        descriptor_json["endpoints"]["transport"],
        transport.to_string()
    );
    assert_eq!(
        descriptor_json["transport_protocols"],
        serde_json::json!(["plain-tcp"])
    );
    assert!(descriptor_json["auth"]["control_token"].is_boolean());
    if descriptor_json["auth"]["control_token"] == true {
        assert_eq!(
            descriptor_json["auth"]["token_metadata"]["scope"],
            "daemon-control-transport"
        );
        assert_eq!(descriptor_json["auth"]["token_generation"], 1);
    }

    let peers = run_control(&endpoint, &["peers"]);
    assert!(peers.status.success());
    let peers_json: serde_json::Value = serde_json::from_slice(&peers.stdout).expect("peers json");
    assert_eq!(peers_json["ok"], true);
    assert_eq!(peers_json["peers"].as_array().expect("peer array").len(), 0);

    let nodes = run_control(&endpoint, &["nodes"]);
    assert!(nodes.status.success());
    let nodes_json: serde_json::Value = serde_json::from_slice(&nodes.stdout).expect("nodes json");
    assert_eq!(nodes_json["ok"], true);
    assert_eq!(nodes_json["kind"], "nodes");
    assert_eq!(nodes_json["nodes"][0]["id"], "current");
    assert_eq!(nodes_json["nodes"][0]["state"], "running");
    assert!(
        nodes_json["nodes"][0]["capabilities"]
            .as_array()
            .expect("capabilities")
            .contains(&serde_json::json!("peer_ensure"))
    );

    let jobs = run_control(&endpoint, &["jobs"]);
    assert!(jobs.status.success());
    let jobs_json: serde_json::Value = serde_json::from_slice(&jobs.stdout).expect("jobs json");
    assert_eq!(jobs_json["ok"], true);
    assert_eq!(jobs_json["jobs"].as_array().expect("jobs array").len(), 0);

    let ensured = run_control(&endpoint, &["node-ensure", "--scope", "session"]);
    assert!(ensured.status.success());
    let ensured_json: serde_json::Value =
        serde_json::from_slice(&ensured.stdout).expect("node ensure json");
    assert_eq!(ensured_json["ok"], true);
    assert_eq!(ensured_json["requested_scope"], "session");
    assert_eq!(ensured_json["next_action"], "reuse_current_daemon");

    stop_child(child, &endpoint);
    let _ = fs::remove_file(routes_path);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn node_daemon_reports_missing_peer_forget_as_json_error() {
    let endpoint_addr = free_addr();
    let endpoint = format!("tcp://{endpoint_addr}");
    let transport = free_addr();
    let routes_path = route_store_path("peer-forget-routes");
    let home = temp_dir("peer-forget-home");
    let child = start_daemon_with_home(&endpoint, transport, &routes_path, &home);
    wait_for_status(&endpoint);

    let output = run_control(&endpoint, &["peer-forget", "missing"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json error");
    assert_eq!(json["ok"], false);
    assert!(json["error"].as_str().unwrap().contains("missing"));

    stop_child(child, &endpoint);
    let _ = fs::remove_file(routes_path);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn node_daemon_reports_missing_peer_bootstrap_payload_as_json_error() {
    let endpoint_addr = free_addr();
    let endpoint = format!("tcp://{endpoint_addr}");
    let transport = free_addr();
    let routes_path = route_store_path("peer-bootstrap-routes");
    let home = temp_dir("peer-bootstrap-home");
    let child = start_daemon_with_home(&endpoint, transport, &routes_path, &home);
    wait_for_status(&endpoint);

    let output = run_control(&endpoint, &["send", r#"{"cmd":"peer_bootstrap"}"#]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json error");
    assert_eq!(json["ok"], false);
    assert!(json["error"].as_str().unwrap().contains("bootstrap args"));

    stop_child(child, &endpoint);
    let _ = fs::remove_file(routes_path);
    let _ = fs::remove_dir_all(home);
}
