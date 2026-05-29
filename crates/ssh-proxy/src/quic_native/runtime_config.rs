use anyhow::Result;

use crate::{cli, peer_transport};

pub(super) fn route_id(args: &cli::ProxyArgs) -> String {
    format!("quic-native:{}:{}", args.listen, args.remote_name)
}

pub(super) fn quic_options_from_proxy_args(
    args: &cli::ProxyArgs,
) -> Result<peer_transport::QuicTransportOptions> {
    peer_transport::QuicTransportOptions::new(
        args.quic_max_bidi_streams,
        args.quic_stream_receive_window,
        args.quic_receive_window,
        args.quic_keep_alive_interval_secs,
        args.quic_idle_timeout_secs,
    )
}

pub(super) fn local_node_name() -> String {
    format!(
        "{}@{}",
        whoami::username().unwrap_or_else(|_| "unknown".to_string()),
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    )
}
