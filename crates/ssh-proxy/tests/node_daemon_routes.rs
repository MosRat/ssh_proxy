mod support;

use std::{fs, net::TcpListener, process::Stdio, thread, time::Duration};

use support::node_daemon::*;

#[test]
fn node_daemon_persists_restores_and_forgets_routes() {
    let control = free_addr();
    let transport = free_addr();
    let listen = free_addr();
    let endpoint = format!("tcp://{control}");
    let routes_path = route_store_path("routes");
    let child = start_daemon(&endpoint, transport, &routes_path);

    wait_for_status(&endpoint);

    let started = run_control(
        &endpoint,
        &[
            "forward",
            "127.0.0.1",
            "--id",
            "persisted",
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
    assert_eq!(started_json["id"], "persisted");
    assert_eq!(started_json["owner"], "local");
    assert_eq!(started_json["direction"], "forward");
    assert_eq!(started_json["listen"], listen.to_string());
    assert!(started_json["peer"].as_str().is_some());
    assert_eq!(started_json["persist"], true);

    let planned = run_control(
        &endpoint,
        &[
            "route-plan",
            "127.0.0.1",
            "--direction",
            "local-uses-remote",
            "--port",
            &listen.port().to_string(),
            "--remote-tls",
            &transport.to_string(),
        ],
    );
    assert!(planned.status.success());
    let planned_json: serde_json::Value =
        serde_json::from_slice(&planned.stdout).expect("route plan json");
    assert_eq!(planned_json["ok"], true);
    assert_eq!(planned_json["plan"]["mode"], "local-forward");
    assert_eq!(
        planned_json["plan"]["preflight"]["kind"],
        "local-direct-transport-probe"
    );

    let store_text = fs::read_to_string(&routes_path).expect("route store written");
    assert!(store_text.contains("persisted"));

    stop_child(child, &endpoint);

    let control = free_addr();
    let transport = free_addr();
    let endpoint = format!("tcp://{control}");
    let child = start_daemon(&endpoint, transport, &routes_path);
    wait_for_status(&endpoint);

    let routes = run_control(&endpoint, &["routes"]);
    assert!(routes.status.success());
    let routes_json: serde_json::Value =
        serde_json::from_slice(&routes.stdout).expect("routes json");
    assert_eq!(routes_json["ok"], true);
    assert_eq!(routes_json["routes"][0]["id"], "persisted");
    assert_eq!(routes_json["routes"][0]["persist"], true);
    assert_eq!(routes_json["routes"][0]["managed_by"], "current-daemon");
    assert_eq!(routes_json["routes"][0]["job_id"], "route:persisted");
    assert_eq!(
        routes_json["routes"][0]["readiness"]["job_id"],
        "route:persisted"
    );
    assert_eq!(routes_json["routes"][0]["readiness"]["phase"], "ready");
    assert_eq!(routes_json["routes"][0]["readiness"]["next_action"], "none");
    assert_eq!(
        routes_json["routes"][0]["runtime"]["selected_transport"],
        "auto"
    );
    assert_eq!(
        routes_json["routes"][0]["runtime"]["transport_pool_size"],
        1
    );
    assert_eq!(routes_json["routes"][0]["link"]["transport_pool_size"], 1);
    assert!(routes_json["routes"][0]["link"]["active_bridges"].is_number());
    let link = routes_json["routes"][0]["link"]
        .as_object()
        .expect("link status object");
    assert!(link.contains_key("selected_protocol"));
    assert_eq!(
        routes_json["routes"][0]["link"]["workers"]
            .as_array()
            .expect("worker status array")
            .len(),
        1
    );
    assert_eq!(routes_json["routes"][0]["link"]["workers"][0]["slot"], 0);
    assert!(
        routes_json["routes"][0]["link"]["workers"][0]
            .as_object()
            .expect("worker status object")
            .contains_key("selected_protocol")
    );
    assert!(
        routes_json["routes"][0]["link"]["workers"][0]
            .as_object()
            .expect("worker status object")
            .contains_key("last_successful_protocol")
    );
    assert!(
        routes_json["routes"][0]["link"]["workers"][0]
            .as_object()
            .expect("worker status object")
            .contains_key("last_failure_ago_secs")
    );
    assert!(routes_json["routes"][0]["link"]["workers"][0]["connect_attempts"].is_number());
    assert!(routes_json["routes"][0]["link"]["workers"][0]["state"].is_string());
    assert!(routes_json["routes"][0]["link"]["workers"][0]["retry_count"].is_number());
    assert!(routes_json["routes"][0]["link"]["workers"][0]["active_streams"].is_number());
    assert!(routes_json["routes"][0]["link"]["workers"][0]["bytes_client_to_remote"].is_number());
    assert!(routes_json["routes"][0]["link"]["workers"][0]["bytes_remote_to_client"].is_number());
    assert!(routes_json["routes"][0]["link"]["connect_attempts"].is_number());
    assert!(routes_json["routes"][0]["link"]["healthy_workers"].is_number());
    assert!(routes_json["routes"][0]["link"]["degraded_workers"].is_number());
    assert!(routes_json["routes"][0]["link"]["reconnecting_workers"].is_number());
    assert!(routes_json["routes"][0]["link"]["tcp_open_attempts"].is_number());
    assert!(routes_json["routes"][0]["link"]["tcp_open_successes"].is_number());
    assert!(routes_json["routes"][0]["link"]["tcp_open_failures"].is_number());
    assert!(routes_json["routes"][0]["link"]["candidate_failures"].is_array());
    assert_eq!(
        routes_json["routes"][0]["link"]["bytes_client_to_remote"],
        0
    );
    assert_eq!(
        routes_json["routes"][0]["link"]["bytes_remote_to_client"],
        0
    );
    assert!(routes_json["routes"][0]["link"]["spx_frame_write_batches"].is_number());
    assert!(routes_json["routes"][0]["link"]["spx_frame_write_flushes"].is_number());
    assert!(routes_json["routes"][0]["link"]["spx_frame_write_vectored_writes"].is_number());
    assert!(routes_json["routes"][0]["link"]["spx_frame_read_frames"].is_number());
    assert!(routes_json["routes"][0]["link"]["spx_tcp_relay_samples"].is_number());
    assert!(routes_json["routes"][0]["link"]["ssh_direct_channel_open_samples"].is_number());
    assert!(
        routes_json["routes"][0]["link"]["last_ssh_direct_channel_open_latency_ms"].is_null()
            || routes_json["routes"][0]["link"]["last_ssh_direct_channel_open_latency_ms"]
                .is_number()
    );
    assert!(routes_json["routes"][0]["link"]["spx_peer_handshake_samples"].is_number());
    assert!(
        routes_json["routes"][0]["link"]["last_spx_peer_handshake_latency_ms"].is_null()
            || routes_json["routes"][0]["link"]["last_spx_peer_handshake_latency_ms"].is_number()
    );

    let stopped = run_control(&endpoint, &["stop-route", "persisted"]);
    assert!(stopped.status.success());
    let stopped_json: serde_json::Value =
        serde_json::from_slice(&stopped.stdout).expect("stop json");
    assert_eq!(stopped_json["ok"], true);
    assert_eq!(stopped_json["removed_persistent"], true);

    let store_text = fs::read_to_string(&routes_path).expect("route store updated");
    let store_json: serde_json::Value =
        serde_json::from_str(&store_text).expect("route store json");
    assert_eq!(
        store_json["routes"].as_array().expect("routes array").len(),
        0
    );

    stop_child(child, &endpoint);
    let _ = fs::remove_file(routes_path);
}
