use std::{net::SocketAddr, time::Duration};

use anyhow::{Result, anyhow};
use tokio::{io, time};
use tracing::{info, warn};

use crate::{cli, config, deploy, peer_transport, remote};

pub async fn run(args: cli::ReverseArgs, config: config::AppConfig) -> Result<()> {
    warn!(
        "reverse mode exposes the default SOCKS5H listener on the SSH host and uses this machine as egress; for production prefer persistent services on both ends"
    );
    let proxy = reverse_to_proxy_args(&args, &config)?;
    let reconnect = !args.no_reconnect;
    let mut delay = Duration::from_secs(args.reconnect_delay_secs);
    let max_delay =
        Duration::from_secs(args.reconnect_max_delay_secs.max(args.reconnect_delay_secs));

    loop {
        let connect = time::timeout(
            Duration::from_secs(args.connect_timeout_secs.max(1)),
            deploy::open_remote_reverse_socks(&proxy, args.remote_listen),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "reverse bridge connection timed out after {}s",
                args.connect_timeout_secs
            )
        })
        .and_then(|result| result);

        match connect {
            Ok(stream) => {
                delay = Duration::from_secs(args.reconnect_delay_secs);
                info!(
                    remote_listen = %args.remote_listen,
                    "reverse SOCKS bridge connected; local machine is now egress"
                );
                let (reader, writer) = io::split(stream);
                if let Err(err) = remote::run_transport_with_egress_proxy(
                    reader,
                    writer,
                    args.egress_proxy.clone(),
                )
                .await
                {
                    warn!(error = %err, "reverse bridge transport stopped");
                }
            }
            Err(err) => warn!(error = %err, "failed to connect reverse bridge"),
        }

        if !reconnect {
            break;
        }
        warn!(
            next_retry_secs = delay.as_secs(),
            "retrying reverse bridge connection after backoff"
        );
        time::sleep(delay).await;
        delay = (delay * 2).min(max_delay);
    }
    Ok(())
}

fn reverse_to_proxy_args(
    args: &cli::ReverseArgs,
    config: &config::AppConfig,
) -> Result<cli::ProxyArgs> {
    let mut proxy = cli::ProxyArgs {
        target: args.target.clone(),
        listen: SocketAddr::from(([127, 0, 0, 1], 1080)),
        tcp_target: args.tcp_target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: None,
        user: args.user.clone(),
        port: args.port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
        remote_path: args.remote_path.clone(),
        remote_bin: args.remote_bin.clone(),
        deploy: args.deploy,
        remote_os: args.remote_os,
        egress_proxy: None,
        remote_transport: cli::RemoteTransport::Exec,
        remote_tcp: SocketAddr::from(([127, 0, 0, 1], 19080)),
        remote_control: SocketAddr::from(([127, 0, 0, 1], 19081)),
        remote_quic: None,
        allow_plain_tcp: false,
        remote_tls: None,
        remote_ca: None,
        remote_name: "localhost".to_string(),
        remote_client_cert: None,
        remote_client_key: None,
        remote_token: None,
        reconnect_delay_secs: args.reconnect_delay_secs,
        reconnect_max_delay_secs: args.reconnect_max_delay_secs,
        connect_timeout_secs: args.connect_timeout_secs,
        transport_pool_size: 1,
        pool_policy: Some("large".to_string()),
        workload_hint: Some(cli::RouteWorkloadHint::Large),
        quic_max_bidi_streams: peer_transport::QUIC_MAX_BIDI_STREAMS,
        quic_stream_receive_window: peer_transport::QUIC_STREAM_RECEIVE_WINDOW,
        quic_receive_window: peer_transport::QUIC_RECEIVE_WINDOW,
        quic_keep_alive_interval_secs: peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS,
        quic_idle_timeout_secs: peer_transport::QUIC_IDLE_TIMEOUT_SECS,
        ssh_session_pool_size: None,
        ssh_session_pool_source: None,
        ssh_session_pool_reason: None,
        ssh_session_pool_warning: None,
        transport_pool_source: Some("fixed".to_string()),
        transport_pool_reason: Some(
            "reverse helper uses one SSH exec bridge; transport pooling applies to daemon forward routes"
                .to_string(),
        ),
        transport_selection_source: Some("fixed".to_string()),
        transport_selection_reason: Some(
            "reverse helper is established over SSH exec and does not use peer transport auto selection"
                .to_string(),
        ),
        preflight_recommended_fallback: None,
        preflight_selected_reason: None,
        preflight_repair_hint: None,
        preflight_candidate_failures: Vec::new(),
        no_reconnect: args.no_reconnect,
        control_listen: None,
    };
    crate::config::apply_proxy_defaults(config, &mut proxy, None)?;
    proxy.remote_transport = cli::RemoteTransport::Exec;
    proxy.remote_tcp = SocketAddr::from(([127, 0, 0, 1], 19080));
    proxy.remote_control = SocketAddr::from(([127, 0, 0, 1], 19081));
    proxy.remote_token = None;
    Ok(proxy)
}
