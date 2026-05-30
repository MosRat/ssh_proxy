mod support;

use std::{fs, net::SocketAddr, path::PathBuf, process::Stdio, thread, time::Duration};

use support::node_daemon::*;

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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
) -> (ChildGuard, ChildGuard, String, SocketAddr, PathBuf, String) {
    start_plain_tcp_proxy_with_target(remote_transport, allow_plain_tcp, token, None)
}

fn start_plain_tcp_proxy_with_target(
    remote_transport: &str,
    allow_plain_tcp: bool,
    token: Option<&str>,
    tcp_target: Option<String>,
) -> (ChildGuard, ChildGuard, String, SocketAddr, PathBuf, String) {
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
        .map(ChildGuard::new)
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
