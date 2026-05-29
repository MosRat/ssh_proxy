use std::{net::SocketAddr, time::Duration};

use serde_json::{Value, json};
use tokio::{net::TcpStream, time};

use crate::{cli, peer_transport, quic_stream};

use super::response::{candidate_failures, is_direct_probe_protocol, refresh_decision_chain};
use super::transport::{ssh_data_plane_reason, ssh_mode_reason};

pub(crate) async fn add_local_transport_probe_results(
    plan: &mut Value,
    forward: &mut cli::NodeForwardArgs,
) {
    let timeout = Duration::from_millis(750);
    let mut results = Vec::new();

    if let Some(addr) = forward.remote_quic {
        results.push(probe_quic_endpoint(forward, addr, timeout).await);
    }

    if let Some(addr) = forward.remote_tls {
        results.push(probe_tcp_endpoint("tls-tcp", addr, timeout).await);
    }

    if forward.allow_plain_tcp {
        results.push(probe_tcp_endpoint("plain-tcp", forward.remote_tcp, timeout).await);
    }

    results.push(json!({
        "protocol": "ssh-direct-tcpip",
        "endpoint": forward.remote_tcp.to_string(),
        "reachable": Value::Null,
        "status": "not-probed",
        "message": "SSH direct-tcpip reachability follows the SSH session and may work even when direct private endpoints do not",
    }));
    results.push(json!({
        "protocol": "ssh-exec",
        "endpoint": Value::Null,
        "reachable": Value::Null,
        "status": "not-probed",
        "message": "SSH exec fallback is validated when the route connects over SSH",
    }));

    let candidate_failures = candidate_failures(&results);
    let direct_failures = candidate_failures.len();
    let direct_successes = results
        .iter()
        .filter(|result| {
            is_direct_probe_protocol(result["protocol"].as_str()) && result["reachable"] == true
        })
        .count();
    let recommended_fallback = if direct_failures > 0 && direct_successes == 0 {
        Some("ssh-native")
    } else {
        None
    };
    let selected_reason = if recommended_fallback.is_some() {
        "all probed direct peer transports failed; SSH fallback is recommended before starting the route"
    } else if direct_successes > 0 {
        "at least one direct peer transport was reachable before route start"
    } else {
        "no failing direct peer transport was observed before route start"
    };
    let repair_hint = if recommended_fallback.is_some() {
        "use ssh-native fallback, or publish a peer endpoint reachable from this client"
    } else if candidate_failures.is_empty() {
        "none"
    } else {
        "publish a reachable peer endpoint, adjust firewall/NAT, or switch to an SSH fallback transport"
    };
    forward.preflight_recommended_fallback = recommended_fallback.map(str::to_string);
    forward.preflight_selected_reason = Some(selected_reason.to_string());
    forward.preflight_repair_hint = Some(repair_hint.to_string());
    forward.preflight_candidate_failures = candidate_failures.clone();

    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "preflight".to_string(),
            json!({
                "kind": "local-direct-transport-probe",
                "timeout_ms": timeout.as_millis(),
                "results": results,
                "candidate_failures": candidate_failures,
                "recommended_fallback": recommended_fallback,
                "selected_reason": selected_reason,
                "repair_hint": repair_hint,
            }),
        );
    }
    refresh_decision_chain(plan);
}

pub(crate) fn apply_local_forward_fallback(
    forward: &mut cli::NodeForwardArgs,
    plan: &mut Value,
) -> Option<String> {
    let recommended = plan
        .pointer("/preflight/recommended_fallback")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let Some(recommended) = recommended else {
        return None;
    };
    let source = forward
        .transport_selection_source
        .as_deref()
        .unwrap_or("unknown");
    let may_override = forward.remote_transport == cli::RemoteTransport::Auto
        || matches!(source, "topology" | "benchmark-tuned default");
    if !may_override {
        return None;
    }
    if recommended != "ssh-native" && recommended != "ssh-direct-tcpip" {
        return None;
    }
    forward.remote_transport = cli::RemoteTransport::SshNative;
    let reason =
        "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
            .to_string();
    forward.transport_selection_source = Some("route-preflight".to_string());
    forward.transport_selection_reason = Some(reason.clone());
    if let Some(object) = plan.as_object_mut() {
        object.insert("selected_transport".to_string(), json!("ssh-native"));
        object.insert(
            "transport_selection_source".to_string(),
            json!("route-preflight"),
        );
        object.insert(
            "transport_selection_reason".to_string(),
            json!(reason.clone()),
        );
        object.insert("ssh_mode".to_string(), json!("native-direct-tcpip"));
        object.insert(
            "ssh_mode_reason".to_string(),
            ssh_mode_reason(cli::RemoteTransport::SshNative),
        );
        object.insert(
            "ssh_data_plane_reason".to_string(),
            ssh_data_plane_reason(
                cli::RemoteTransport::SshNative,
                forward.transport_selection_source.as_deref(),
            ),
        );
        object.insert(
            "ssh_session_pool_size".to_string(),
            json!(forward.ssh_session_pool_size.unwrap_or(1)),
        );
        object.insert(
            "ssh_session_pool_source".to_string(),
            json!(
                forward
                    .ssh_session_pool_source
                    .as_deref()
                    .unwrap_or("unknown")
            ),
        );
        object.insert(
            "ssh_session_pool_reason".to_string(),
            json!(
                forward
                    .ssh_session_pool_reason
                    .as_deref()
                    .unwrap_or("unknown")
            ),
        );
        object.insert(
            "ssh_session_pool_warning".to_string(),
            json!(forward.ssh_session_pool_warning.as_deref()),
        );
        object.insert("fallback_reason".to_string(), json!(reason.clone()));
        object.insert(
            "next_action".to_string(),
            json!("using ssh-native fallback; no user action required"),
        );
    }
    refresh_decision_chain(plan);
    Some(reason)
}

async fn probe_quic_endpoint(
    forward: &cli::NodeForwardArgs,
    addr: SocketAddr,
    timeout: Duration,
) -> Value {
    if addr.ip().is_loopback() {
        return json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "loopback QUIC endpoint is local to the caller; probing it here would not prove reachability to the peer daemon",
        });
    }
    let Some(ca) = forward.remote_ca.as_deref() else {
        return json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "QUIC handshake probe requires --remote-ca or profile remote_ca",
        });
    };
    if forward.remote_client_cert.is_some() || forward.remote_client_key.is_some() {
        return json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "QUIC mTLS probing is not implemented yet; use TLS/TCP probing for mTLS routes",
        });
    }

    let roots = match peer_transport::load_cert_chain(ca) {
        Ok(roots) => roots,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("failed to load QUIC probe CA {}: {err:#}", ca.display()),
            });
        }
    };
    let mut endpoint = match quinn::Endpoint::client(SocketAddr::from(([0, 0, 0, 0], 0))) {
        Ok(endpoint) => endpoint,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("failed to create QUIC probe endpoint: {err:#}"),
            });
        }
    };
    let quic_options = match peer_transport::QuicTransportOptions::new(
        forward.quic_max_bidi_streams,
        forward.quic_stream_receive_window,
        forward.quic_receive_window,
        forward.quic_keep_alive_interval_secs,
        forward.quic_idle_timeout_secs,
    ) {
        Ok(options) => options,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("invalid QUIC probe transport config: {err:#}"),
            });
        }
    };
    match peer_transport::quic_client_config(roots, quic_options) {
        Ok(config) => endpoint.set_default_client_config(config),
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "probe-config-error",
                "message": format!("failed to build QUIC probe client config: {err:#}"),
            });
        }
    }

    let connecting = match endpoint.connect(addr, &forward.remote_name) {
        Ok(connecting) => connecting,
        Err(err) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "connect-request-failed",
                "message": err.to_string(),
            });
        }
    };
    let connection = match time::timeout(timeout, connecting).await {
        Ok(Ok(connection)) => connection,
        Ok(Err(err)) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "handshake-failed",
                "message": err.to_string(),
            });
        }
        Err(_) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "timeout",
                "message": format!("QUIC handshake timed out after {} ms", timeout.as_millis()),
            });
        }
    };
    let (send, recv) = match time::timeout(timeout, connection.open_bi()).await {
        Ok(Ok(streams)) => streams,
        Ok(Err(err)) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "stream-open-failed",
                "message": err.to_string(),
            });
        }
        Err(_) => {
            return json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": false,
                "status": "timeout",
                "message": format!("QUIC bidirectional stream open timed out after {} ms", timeout.as_millis()),
            });
        }
    };
    let mut stream = quic_stream::QuicBiStream::with_lifetime(send, recv, connection, endpoint);
    match time::timeout(
        timeout,
        peer_transport::client_handshake(
            &mut stream,
            "route-preflight",
            peer_transport::PeerProtocol::Quic,
        ),
    )
    .await
    {
        Ok(Ok(welcome)) => {
            stream.finish();
            json!({
                "protocol": "quic",
                "endpoint": addr.to_string(),
                "reachable": true,
                "status": "reachable",
                "message": format!("QUIC handshake succeeded before route start; remote node {}", welcome.node),
            })
        }
        Ok(Err(err)) => json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "peer-handshake-failed",
            "message": format!("{err:#}"),
        }),
        Err(_) => json!({
            "protocol": "quic",
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "timeout",
            "message": format!("QUIC peer handshake timed out after {} ms", timeout.as_millis()),
        }),
    }
}

async fn probe_tcp_endpoint(protocol: &str, addr: SocketAddr, timeout: Duration) -> Value {
    if addr.ip().is_loopback() {
        return json!({
            "protocol": protocol,
            "endpoint": addr.to_string(),
            "reachable": Value::Null,
            "status": "skipped",
            "message": "loopback endpoint is local to the caller; probing it here would not prove reachability to the peer daemon",
        });
    }

    match time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            json!({
                "protocol": protocol,
                "endpoint": addr.to_string(),
                "reachable": true,
                "status": "reachable",
                "message": "TCP connect succeeded before route start",
            })
        }
        Ok(Err(err)) => json!({
            "protocol": protocol,
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "connect-failed",
            "message": err.to_string(),
        }),
        Err(_) => json!({
            "protocol": protocol,
            "endpoint": addr.to_string(),
            "reachable": false,
            "status": "timeout",
            "message": format!("TCP connect timed out after {} ms", timeout.as_millis()),
        }),
    }
}
