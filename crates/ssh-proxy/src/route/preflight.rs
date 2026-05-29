use std::{net::SocketAddr, time::Duration};

use serde_json::Value;
use ssh_proxy_core::model::TransportMode;
use ssh_proxy_route::{
    RouteFallbackInput, RoutePreflightInput, RouteProbeResult, SshSessionPoolReport,
    decide_route_fallback, decide_route_preflight,
};
use ssh_proxy_transport::quic::connect_client;
use tokio::{net::TcpStream, time};

use crate::{cli, peer_transport};

use super::response::refresh_decision_chain;

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

    results.push(RouteProbeResult::new(
        "ssh-direct-tcpip",
        Some(forward.remote_tcp.to_string()),
        None,
        "not-probed",
        "SSH direct-tcpip reachability follows the SSH session and may work even when direct private endpoints do not",
    ));
    results.push(RouteProbeResult::new(
        "ssh-exec",
        None,
        None,
        "not-probed",
        "SSH exec fallback is validated when the route connects over SSH",
    ));

    let decision = decide_route_preflight(RoutePreflightInput {
        timeout_ms: timeout.as_millis() as u64,
        results,
    });
    forward.preflight_recommended_fallback = decision.recommended_fallback.clone();
    forward.preflight_selected_reason = Some(decision.selected_reason.clone());
    forward.preflight_repair_hint = Some(decision.repair_hint.clone());
    forward.preflight_candidate_failures = decision.candidate_failures.clone();

    if let Some(object) = plan.as_object_mut() {
        object.insert("preflight".to_string(), decision.to_plan_value());
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
    let decision = decide_route_fallback(RouteFallbackInput {
        recommended_fallback: Some(recommended.as_str()),
        current_transport: TransportMode::from(forward.remote_transport),
        selection_source: forward.transport_selection_source.as_deref(),
    });
    let selected = decision.selected_transport?;
    forward.remote_transport = remote_transport_from_mode(selected)?;
    forward.transport_selection_source = decision.selection_source.clone();
    forward.transport_selection_reason = decision.selection_reason.clone();

    let ssh_session_pool = ssh_session_pool_report(forward);
    decision.apply_to_plan(plan, ssh_session_pool.as_ref());
    decision.reason().map(ToOwned::to_owned)
}

async fn probe_quic_endpoint(
    forward: &cli::NodeForwardArgs,
    addr: SocketAddr,
    timeout: Duration,
) -> RouteProbeResult {
    if addr.ip().is_loopback() {
        return RouteProbeResult::new(
            "quic",
            Some(addr.to_string()),
            None,
            "skipped",
            "loopback QUIC endpoint is local to the caller; probing it here would not prove reachability to the peer daemon",
        );
    }
    let Some(ca) = forward.remote_ca.as_deref() else {
        return RouteProbeResult::new(
            "quic",
            Some(addr.to_string()),
            None,
            "skipped",
            "QUIC handshake probe requires --remote-ca or profile remote_ca",
        );
    };
    if forward.remote_client_cert.is_some() || forward.remote_client_key.is_some() {
        return RouteProbeResult::new(
            "quic",
            Some(addr.to_string()),
            None,
            "skipped",
            "QUIC mTLS probing is not implemented yet; use TLS/TCP probing for mTLS routes",
        );
    }

    let roots = match peer_transport::load_cert_chain(ca) {
        Ok(roots) => roots,
        Err(err) => {
            return RouteProbeResult::new(
                "quic",
                Some(addr.to_string()),
                Some(false),
                "probe-config-error",
                format!("failed to load QUIC probe CA {}: {err:#}", ca.display()),
            );
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
            return RouteProbeResult::new(
                "quic",
                Some(addr.to_string()),
                Some(false),
                "probe-config-error",
                format!("invalid QUIC probe transport config: {err:#}"),
            );
        }
    };
    let connection = match connect_client(
        addr,
        &forward.remote_name,
        roots,
        quic_options,
        timeout,
        format!("connect QUIC preflight endpoint {addr}"),
    )
    .await
    {
        Ok(connection) => connection,
        Err(err) => {
            return RouteProbeResult::new(
                "quic",
                Some(addr.to_string()),
                Some(false),
                "handshake-failed",
                format!("{err:#}"),
            );
        }
    };
    let mut stream = match connection
        .open_bi(timeout, "open QUIC preflight stream")
        .await
    {
        Ok(stream) => stream,
        Err(err) => {
            return RouteProbeResult::new(
                "quic",
                Some(addr.to_string()),
                Some(false),
                "stream-open-failed",
                format!("{err:#}"),
            );
        }
    };
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
            RouteProbeResult::new(
                "quic",
                Some(addr.to_string()),
                Some(true),
                "reachable",
                format!(
                    "QUIC handshake succeeded before route start; remote node {}",
                    welcome.node
                ),
            )
        }
        Ok(Err(err)) => RouteProbeResult::new(
            "quic",
            Some(addr.to_string()),
            Some(false),
            "peer-handshake-failed",
            format!("{err:#}"),
        ),
        Err(_) => RouteProbeResult::new(
            "quic",
            Some(addr.to_string()),
            Some(false),
            "timeout",
            format!(
                "QUIC peer handshake timed out after {} ms",
                timeout.as_millis()
            ),
        ),
    }
}

async fn probe_tcp_endpoint(
    protocol: &str,
    addr: SocketAddr,
    timeout: Duration,
) -> RouteProbeResult {
    if addr.ip().is_loopback() {
        return RouteProbeResult::new(
            protocol,
            Some(addr.to_string()),
            None,
            "skipped",
            "loopback endpoint is local to the caller; probing it here would not prove reachability to the peer daemon",
        );
    }

    match time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            RouteProbeResult::new(
                protocol,
                Some(addr.to_string()),
                Some(true),
                "reachable",
                "TCP connect succeeded before route start",
            )
        }
        Ok(Err(err)) => RouteProbeResult::new(
            protocol,
            Some(addr.to_string()),
            Some(false),
            "connect-failed",
            err.to_string(),
        ),
        Err(_) => RouteProbeResult::new(
            protocol,
            Some(addr.to_string()),
            Some(false),
            "timeout",
            format!("TCP connect timed out after {} ms", timeout.as_millis()),
        ),
    }
}

fn remote_transport_from_mode(transport: TransportMode) -> Option<cli::RemoteTransport> {
    match transport {
        TransportMode::Auto => Some(cli::RemoteTransport::Auto),
        TransportMode::SshNative => Some(cli::RemoteTransport::SshNative),
        TransportMode::QuicNative => Some(cli::RemoteTransport::QuicNative),
        TransportMode::Quic => Some(cli::RemoteTransport::Quic),
        TransportMode::TlsTcp => Some(cli::RemoteTransport::TlsTcp),
        TransportMode::PlainTcp => Some(cli::RemoteTransport::PlainTcp),
        TransportMode::Exec => Some(cli::RemoteTransport::Exec),
        TransportMode::Tcp => Some(cli::RemoteTransport::Tcp),
    }
}

fn ssh_session_pool_report(forward: &cli::NodeForwardArgs) -> Option<SshSessionPoolReport> {
    matches!(forward.remote_transport, cli::RemoteTransport::SshNative).then(|| {
        SshSessionPoolReport {
            size: forward.ssh_session_pool_size.unwrap_or(1),
            source: forward
                .ssh_session_pool_source
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            reason: forward
                .ssh_session_pool_reason
                .as_deref()
                .unwrap_or("unknown")
                .to_string(),
            warning: forward.ssh_session_pool_warning.clone(),
        }
    })
}
