use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral port");
    listener.local_addr().expect("read local addr")
}

fn ssh_proxy_command() -> Command {
    let home = temp_dir("cmd-home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command.env("SSH_PROXY_HOME", home);
    command
}

fn run_control(endpoint: &str, args: &[&str]) -> std::process::Output {
    let mut command = ssh_proxy_command();
    command.args(["--log", "warn", "node", "control", "--endpoint", endpoint]);
    command.args(args);
    command.output().expect("run node control")
}

fn run_control_with_token(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    let mut command = ssh_proxy_command();
    command.args([
        "--log",
        "warn",
        "node",
        "control",
        "--endpoint",
        endpoint,
        "--token",
        token,
    ]);
    command.args(args);
    command.output().expect("run node control")
}

fn wait_for_status(endpoint: &str) {
    for _ in 0..40 {
        let output = run_control(endpoint, &["status"]);
        if output.status.success()
            && serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|value| value["ok"].as_bool())
                == Some(true)
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("node daemon did not become ready");
}

fn wait_for_status_with_token(endpoint: &str, token: &str) {
    for _ in 0..40 {
        let output = run_control_with_token(endpoint, token, &["status"]);
        if output.status.success()
            && serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|value| value["ok"].as_bool())
                == Some(true)
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("node daemon did not become ready");
}

fn wait_tcp(addr: SocketAddr) {
    for _ in 0..40 {
        if std::net::TcpStream::connect(addr).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("{addr} did not become ready");
}

fn start_daemon(endpoint: &str, transport: SocketAddr, routes_path: &PathBuf) -> Child {
    let home = temp_dir("daemon-home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            endpoint,
            "--transport",
            &transport.to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
        ])
        .env("SSH_PROXY_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start node daemon")
}

fn start_daemon_with_home(
    endpoint: &str,
    transport: SocketAddr,
    routes_path: &PathBuf,
    home: &PathBuf,
) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            endpoint,
            "--transport",
            &transport.to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
        ])
        .env("SSH_PROXY_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start node daemon")
}

fn start_daemon_with_token(
    endpoint: &str,
    transport: SocketAddr,
    routes_path: &PathBuf,
    token: &str,
    home: &PathBuf,
) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            endpoint,
            "--transport",
            &transport.to_string(),
            "--token",
            token,
            "--routes-path",
            &routes_path.display().to_string(),
        ])
        .env("SSH_PROXY_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start node daemon")
}

fn stop_child(mut child: Child, endpoint: &str) {
    let _ = run_control(endpoint, &["shutdown"]);
    for _ in 0..20 {
        if child.try_wait().expect("poll daemon").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn stop_child_with_token(mut child: Child, endpoint: &str, token: &str) {
    let _ = run_control_with_token(endpoint, token, &["shutdown"]);
    for _ in 0..20 {
        if child.try_wait().expect("poll daemon").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn route_store_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("ssh_proxy-{name}-{nanos}.json"))
}

fn temp_path(name: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("ssh_proxy-{name}-{nanos}.{ext}"))
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ssh_proxy-{name}-{nanos}"));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

fn raw_control_request(addr: SocketAddr, request: &str) -> serde_json::Value {
    let mut stream = std::net::TcpStream::connect(addr).expect("connect raw control");
    stream
        .write_all(format!("{request}\n").as_bytes())
        .expect("write raw control");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown write");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read raw control");
    serde_json::from_str(&response).expect("json response")
}

fn start_http_server(body: &'static str) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind http");
    let addr = listener.local_addr().expect("http addr");
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (addr, handle)
}

fn start_http_server_many(
    body: &'static str,
    max_requests: usize,
) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind http");
    listener
        .set_nonblocking(true)
        .expect("set http listener nonblocking");
    let addr = listener.local_addr().expect("http addr");
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut served = 0;
        while served < max_requests && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0_u8; 1024];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    served += 1;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if served > 0 {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (addr, handle)
}

fn socks_get(socks: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(socks).expect("connect socks");
    stream.write_all(&[5, 1, 0]).expect("socks hello");
    let mut hello = [0_u8; 2];
    stream.read_exact(&mut hello).expect("socks hello response");
    assert_eq!(hello, [5, 0]);
    let mut request = vec![5, 1, 0, 1];
    match target.ip() {
        std::net::IpAddr::V4(ip) => request.extend_from_slice(&ip.octets()),
        std::net::IpAddr::V6(_) => panic!("test uses ipv4"),
    }
    request.extend_from_slice(&target.port().to_be_bytes());
    stream.write_all(&request).expect("socks connect");
    let mut reply = [0_u8; 10];
    stream.read_exact(&mut reply).expect("socks reply");
    assert_eq!(reply[1], 0, "socks connect failed: {reply:?}");
    stream
        .write_all(b"GET / HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n\r\n")
        .expect("http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

fn http_connect_get(proxy: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect proxy");
    let connect =
        format!("CONNECT {target} HTTP/1.1\r\nhost: {target}\r\nconnection: keep-alive\r\n\r\n");
    stream.write_all(connect.as_bytes()).expect("http connect");
    let mut response = Vec::new();
    let mut one = [0_u8; 1];
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        stream.read_exact(&mut one).expect("connect response");
        response.push(one[0]);
    }
    let response_text = String::from_utf8(response).expect("connect response utf8");
    assert!(response_text.starts_with("HTTP/1.1 200"), "{response_text}");
    stream
        .write_all(b"GET / HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n\r\n")
        .expect("http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

fn http_absolute_get(proxy: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect proxy");
    let request = format!(
        "GET http://{target}/via-http-proxy?x=1 HTTP/1.1\r\nhost: {target}\r\nconnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("absolute http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

fn fixed_tcp_http_get(proxy: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect fixed tcp proxy");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set fixed tcp read timeout");
    stream
        .write_all(b"GET /fixed HTTP/1.1\r\nhost: fixed-target\r\nconnection: close\r\n\r\n")
        .expect("fixed target http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

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

#[test]
fn tls_transport_can_proxy_without_ssh_client_path() {
    run_tls_proxy_case("tls-tcp", "tls-egress-ok");
}

#[test]
fn auto_transport_prefers_configured_tls_before_ssh_fallback() {
    run_tls_proxy_case("auto", "tls-auto-egress-ok");
}

#[test]
fn quic_transport_can_proxy_without_ssh_client_path() {
    run_quic_proxy_case("quic", "quic-egress-ok");
}

#[test]
fn quic_native_transport_can_proxy_without_ssh_client_path() {
    run_quic_proxy_case("quic-native", "quic-native-egress-ok");
}

#[test]
fn quic_native_fixed_tcp_target_can_proxy_to_specific_port() {
    run_quic_proxy_fixed_target_case("quic-native", "quic-native-fixed-egress-ok");
}

#[test]
fn auto_transport_prefers_configured_quic_before_tls_and_ssh() {
    run_quic_proxy_case("auto", "quic-auto-egress-ok");
}

fn run_quic_proxy_case(remote_transport: &str, body: &'static str) {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("generate cert");
    let cert_path = temp_path("quic-cert", "pem");
    let key_path = temp_path("quic-key", "pem");
    fs::write(&cert_path, certified.cert.pem()).expect("write cert");
    fs::write(&key_path, certified.signing_key.serialize_pem()).expect("write key");

    let control = free_addr();
    let quic = free_addr();
    let routes_path = route_store_path("quic-routes");
    let endpoint = format!("tcp://{control}");
    let daemon = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            &endpoint,
            "--quic-transport",
            &quic.to_string(),
            "--tls-cert",
            &cert_path.display().to_string(),
            "--tls-key",
            &key_path.display().to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
            "--no-route-autostart",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start quic daemon");
    wait_for_status(&endpoint);
    thread::sleep(Duration::from_millis(200));

    let socks = free_addr();
    let proxy_control = free_addr();
    let proxy_control_text = proxy_control.to_string();
    let mut proxy = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "proxy",
            "unused-for-quic",
            "--listen",
            &socks.to_string(),
            "--remote-transport",
            remote_transport,
            "--remote-quic",
            &quic.to_string(),
            "--remote-ca",
            &cert_path.display().to_string(),
            "--remote-name",
            "localhost",
            "--transport-pool-size",
            if remote_transport == "quic-native" {
                "2"
            } else {
                "1"
            },
            "--control-listen",
            &proxy_control_text,
            "--no-reconnect",
            "--deploy",
            "never",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start quic proxy");
    wait_tcp(socks);

    let (http, http_handle) = start_http_server_many(body, 20);
    let response = socks_get(socks, http);
    assert!(response.contains(body));
    if remote_transport == "quic-native" {
        let status = raw_control_request(proxy_control, "status");
        assert_eq!(status["selected_protocol"], "quic-native");
        assert_eq!(status["quic_mode"], "native-per-flow");
        assert_eq!(status["quic_connection_pool_size"], 2);
        assert_eq!(status["active_quic_connections"], 2);
        assert!(status["quic_connections"].is_array());
        assert_eq!(status["quic_flow_resets"], 0);
        assert!(
            status["quic_flow_graceful_closes"].as_u64().unwrap_or(0) >= 1,
            "{status}"
        );
        assert!(
            status["quic_flow_first_byte_samples"].as_u64().unwrap_or(0) >= 1,
            "{status}"
        );
        assert!(
            status["last_quic_flow_first_byte_latency_ms"].is_number(),
            "{status}"
        );
    }

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child(daemon, &endpoint);
    let _ = http_handle.join();
    let _ = fs::remove_file(cert_path);
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(routes_path);
}

fn run_quic_proxy_fixed_target_case(remote_transport: &str, body: &'static str) {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("generate cert");
    let cert_path = temp_path("quic-fixed-cert", "pem");
    let key_path = temp_path("quic-fixed-key", "pem");
    fs::write(&cert_path, certified.cert.pem()).expect("write cert");
    fs::write(&key_path, certified.signing_key.serialize_pem()).expect("write key");

    let control = free_addr();
    let quic = free_addr();
    let routes_path = route_store_path("quic-fixed-routes");
    let endpoint = format!("tcp://{control}");
    let daemon = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            &endpoint,
            "--quic-transport",
            &quic.to_string(),
            "--tls-cert",
            &cert_path.display().to_string(),
            "--tls-key",
            &key_path.display().to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
            "--no-route-autostart",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start quic fixed daemon");
    wait_for_status(&endpoint);
    thread::sleep(Duration::from_millis(200));

    let (http, http_handle) = start_http_server_many(body, 20);
    let tcp_target = http.to_string();
    let socks = free_addr();
    let mut proxy = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "proxy",
            "unused-for-quic-fixed",
            "--listen",
            &socks.to_string(),
            "--remote-transport",
            remote_transport,
            "--remote-quic",
            &quic.to_string(),
            "--remote-ca",
            &cert_path.display().to_string(),
            "--remote-name",
            "localhost",
            "--tcp-target",
            &tcp_target,
            "--no-reconnect",
            "--deploy",
            "never",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start quic fixed proxy");
    wait_tcp(socks);

    let mut response = String::new();
    for _ in 0..20 {
        response = fixed_tcp_http_get(socks);
        if response.contains(body) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let passed = response.contains(body);

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child(daemon, &endpoint);
    let _ = http_handle.join();
    let _ = fs::remove_file(cert_path);
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(routes_path);
    assert!(
        passed,
        "unexpected QUIC-native fixed TCP response: {response:?}"
    );
}

#[test]
fn mtls_transport_can_proxy_with_client_certificate() {
    let server =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("server cert");
    let client =
        rcgen::generate_simple_self_signed(vec!["client".to_string()]).expect("client cert");
    let server_cert_path = temp_path("mtls-server-cert", "pem");
    let server_key_path = temp_path("mtls-server-key", "pem");
    let client_cert_path = temp_path("mtls-client-cert", "pem");
    let client_key_path = temp_path("mtls-client-key", "pem");
    fs::write(&server_cert_path, server.cert.pem()).expect("write server cert");
    fs::write(&server_key_path, server.signing_key.serialize_pem()).expect("write server key");
    fs::write(&client_cert_path, client.cert.pem()).expect("write client cert");
    fs::write(&client_key_path, client.signing_key.serialize_pem()).expect("write client key");

    let control = free_addr();
    let tls = free_addr();
    let routes_path = route_store_path("mtls-routes");
    let endpoint = format!("tcp://{control}");
    let daemon = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            &endpoint,
            "--tls-transport",
            &tls.to_string(),
            "--tls-cert",
            &server_cert_path.display().to_string(),
            "--tls-key",
            &server_key_path.display().to_string(),
            "--tls-client-ca",
            &client_cert_path.display().to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
            "--no-route-autostart",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start mtls daemon");
    wait_for_status(&endpoint);
    wait_tcp(tls);

    let socks = free_addr();
    let mut proxy = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "proxy",
            "unused-for-mtls",
            "--listen",
            &socks.to_string(),
            "--remote-transport",
            "tls-tcp",
            "--remote-tls",
            &tls.to_string(),
            "--remote-ca",
            &server_cert_path.display().to_string(),
            "--remote-name",
            "localhost",
            "--remote-client-cert",
            &client_cert_path.display().to_string(),
            "--remote-client-key",
            &client_key_path.display().to_string(),
            "--no-reconnect",
            "--deploy",
            "never",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start mtls proxy");
    wait_tcp(socks);

    let (http, http_handle) = start_http_server("mtls-egress-ok");
    let response = socks_get(socks, http);
    assert!(response.contains("mtls-egress-ok"));

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child(daemon, &endpoint);
    let _ = http_handle.join();
    let _ = fs::remove_file(server_cert_path);
    let _ = fs::remove_file(server_key_path);
    let _ = fs::remove_file(client_cert_path);
    let _ = fs::remove_file(client_key_path);
    let _ = fs::remove_file(routes_path);
}

#[test]
fn plain_tcp_transport_can_proxy_without_ssh_client_path() {
    run_plain_tcp_proxy_case("plain-tcp", false, None, "plain-egress-ok");
}

#[test]
fn fixed_tcp_target_can_proxy_to_specific_port() {
    let (http, http_handle) = start_http_server_many("fixed-tcp-target-ok", 20);
    let (daemon, mut proxy, endpoint, socks, routes_path, token) =
        start_plain_tcp_proxy_with_target("plain-tcp", false, None, Some(http.to_string()));

    let mut response = String::new();
    for _ in 0..20 {
        response = fixed_tcp_http_get(socks);
        if response.contains("fixed-tcp-target-ok") {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let passed = response.contains("fixed-tcp-target-ok");

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child_with_token(daemon, &endpoint, &token);
    let _ = http_handle.join();
    let _ = fs::remove_file(routes_path);
    assert!(passed, "unexpected fixed TCP response: {response:?}");
}

#[test]
fn auto_transport_uses_plain_tcp_only_when_explicitly_allowed() {
    run_plain_tcp_proxy_case("auto", true, Some("route-token"), "plain-auto-egress-ok");
}

#[test]
fn unified_listener_accepts_http_connect_and_socks5h_on_same_port() {
    let (daemon, mut proxy, endpoint, socks, routes_path, token) =
        start_plain_tcp_proxy("plain-tcp", false, None);

    let (http, http_handle) = start_http_server("http-connect-egress-ok");
    let response = http_connect_get(socks, http);
    assert!(response.contains("http-connect-egress-ok"));
    let _ = http_handle.join();

    let (http, http_handle) = start_http_server("socks-same-port-egress-ok");
    let response = socks_get(socks, http);
    assert!(response.contains("socks-same-port-egress-ok"));
    let _ = http_handle.join();

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child_with_token(daemon, &endpoint, &token);
    let _ = fs::remove_file(routes_path);
}

#[test]
fn unified_listener_accepts_http_absolute_form_proxy_requests() {
    let (daemon, mut proxy_child, endpoint, proxy, routes_path, token) =
        start_plain_tcp_proxy("plain-tcp", false, None);

    let (http, http_handle) = start_http_server("http-absolute-egress-ok");
    let response = http_absolute_get(proxy, http);
    assert!(response.contains("http-absolute-egress-ok"));

    let _ = proxy_child.kill();
    let _ = proxy_child.wait();
    stop_child_with_token(daemon, &endpoint, &token);
    let _ = http_handle.join();
    let _ = fs::remove_file(routes_path);
}

fn run_plain_tcp_proxy_case(
    remote_transport: &str,
    allow_plain_tcp: bool,
    token: Option<&str>,
    body: &'static str,
) {
    let (daemon, mut proxy, endpoint, socks, routes_path, token) =
        start_plain_tcp_proxy(remote_transport, allow_plain_tcp, token);

    let (http, http_handle) = start_http_server(body);
    let response = socks_get(socks, http);
    assert!(response.contains(body));

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child_with_token(daemon, &endpoint, &token);
    let _ = http_handle.join();
    let _ = fs::remove_file(routes_path);
}

fn start_plain_tcp_proxy(
    remote_transport: &str,
    allow_plain_tcp: bool,
    token: Option<&str>,
) -> (Child, Child, String, SocketAddr, PathBuf, String) {
    start_plain_tcp_proxy_with_target(remote_transport, allow_plain_tcp, token, None)
}

fn start_plain_tcp_proxy_with_target(
    remote_transport: &str,
    allow_plain_tcp: bool,
    token: Option<&str>,
    tcp_target: Option<String>,
) -> (Child, Child, String, SocketAddr, PathBuf, String) {
    let control = free_addr();
    let transport = free_addr();
    let routes_path = route_store_path("plain-routes");
    let endpoint = format!("tcp://{control}");
    let mut daemon_args = vec![
        "--log".to_string(),
        "warn".to_string(),
        "node".to_string(),
        "daemon".to_string(),
        "--control".to_string(),
        endpoint.clone(),
        "--transport".to_string(),
        transport.to_string(),
        "--routes-path".to_string(),
        routes_path.display().to_string(),
        "--no-route-autostart".to_string(),
    ];
    let effective_token = token.unwrap_or("plain-test-token");
    daemon_args.push("--token".to_string());
    daemon_args.push(effective_token.to_string());
    let daemon = ssh_proxy_command()
        .args(daemon_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start plain tcp daemon");
    wait_for_status_with_token(&endpoint, effective_token);
    wait_tcp(transport);

    let socks = free_addr();
    let mut proxy_args = vec![
        "--log".to_string(),
        "warn".to_string(),
        "proxy".to_string(),
        "unused-for-plain-tcp".to_string(),
        "--listen".to_string(),
        socks.to_string(),
        "--remote-transport".to_string(),
        remote_transport.to_string(),
        "--remote-tcp".to_string(),
        transport.to_string(),
        "--no-reconnect".to_string(),
        "--deploy".to_string(),
        "never".to_string(),
    ];
    if allow_plain_tcp {
        proxy_args.push("--allow-plain-tcp".to_string());
    }
    if let Some(tcp_target) = tcp_target {
        proxy_args.push("--tcp-target".to_string());
        proxy_args.push(tcp_target);
    }
    proxy_args.push("--remote-token".to_string());
    proxy_args.push(effective_token.to_string());
    let proxy = ssh_proxy_command()
        .args(proxy_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start plain tcp proxy");
    wait_tcp(socks);
    (
        daemon,
        proxy,
        endpoint,
        socks,
        routes_path,
        effective_token.to_string(),
    )
}

fn run_tls_proxy_case(remote_transport: &str, body: &'static str) {
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("generate cert");
    let cert_path = temp_path("tls-cert", "pem");
    let key_path = temp_path("tls-key", "pem");
    fs::write(&cert_path, certified.cert.pem()).expect("write cert");
    fs::write(&key_path, certified.signing_key.serialize_pem()).expect("write key");

    let control = free_addr();
    let tls = free_addr();
    let routes_path = route_store_path("tls-routes");
    let endpoint = format!("tcp://{control}");
    let daemon = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            &endpoint,
            "--tls-transport",
            &tls.to_string(),
            "--tls-cert",
            &cert_path.display().to_string(),
            "--tls-key",
            &key_path.display().to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
            "--no-route-autostart",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start tls daemon");
    wait_for_status(&endpoint);
    wait_tcp(tls);

    let socks = free_addr();
    let mut proxy = ssh_proxy_command()
        .args([
            "--log",
            "warn",
            "proxy",
            "unused-for-tls",
            "--listen",
            &socks.to_string(),
            "--remote-transport",
            remote_transport,
            "--remote-tls",
            &tls.to_string(),
            "--remote-ca",
            &cert_path.display().to_string(),
            "--remote-name",
            "localhost",
            "--no-reconnect",
            "--deploy",
            "never",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start tls proxy");
    wait_tcp(socks);

    let (http, http_handle) = start_http_server(body);
    let response = socks_get(socks, http);
    assert!(response.contains(body));

    let _ = proxy.kill();
    let _ = proxy.wait();
    stop_child(daemon, &endpoint);
    let _ = http_handle.join();
    let _ = fs::remove_file(cert_path);
    let _ = fs::remove_file(key_path);
    let _ = fs::remove_file(routes_path);
}
