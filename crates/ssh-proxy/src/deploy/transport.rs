use std::net::SocketAddr;

use anyhow::{Result, anyhow};
use ssh_proxy_core::model::{RemotePlatform, TransportMode};
use ssh_proxy_transport::{
    peer_transport::QuicTransportOptions,
    remote_helper::{
        BoxedRemoteStream, OpenedRemoteHelper, RemoteHelperOpenIntent,
        client::{self, RemoteHelperFuture, SshDirectConnector},
    },
};

use crate::{cli, ssh_client};

use super::helper::{
    HelperCapability, ensure_helper, remote_reverse_socks_command, remote_stdio_command,
};

pub async fn open_remote_helper(args: &cli::ProxyArgs) -> Result<OpenedRemoteHelper> {
    let intent = remote_helper_open_intent(args)?;
    let mut connector = ProxySshConnector::new(args);
    client::open_remote_helper(&intent, &local_node_name(), &mut connector).await
}

fn remote_helper_open_intent(args: &cli::ProxyArgs) -> Result<RemoteHelperOpenIntent> {
    Ok(RemoteHelperOpenIntent {
        transport: TransportMode::from(args.remote_transport),
        remote_platform: RemotePlatform::from(args.remote_os),
        remote_tcp: args.remote_tcp,
        remote_quic: args.remote_quic,
        remote_tls: args.remote_tls,
        remote_name: args.remote_name.clone(),
        remote_ca: args.remote_ca.clone(),
        remote_client_cert: args.remote_client_cert.clone(),
        remote_client_key: args.remote_client_key.clone(),
        remote_token: args.remote_token.clone(),
        allow_plain_tcp: args.allow_plain_tcp,
        connect_timeout_secs: args.connect_timeout_secs,
        quic: QuicTransportOptions::new(
            args.quic_max_bidi_streams,
            args.quic_stream_receive_window,
            args.quic_receive_window,
            args.quic_keep_alive_interval_secs,
            args.quic_idle_timeout_secs,
        )?,
    })
}

struct ProxySshConnector<'a> {
    args: &'a cli::ProxyArgs,
    client: Option<ssh_client::Client>,
}

impl<'a> ProxySshConnector<'a> {
    fn new(args: &'a cli::ProxyArgs) -> Self {
        Self { args, client: None }
    }

    async fn client(&mut self) -> Result<&ssh_client::Client> {
        if self.client.is_none() {
            self.client = Some(ssh_client::Client::connect_proxy_args(self.args).await?);
        }
        self.client
            .as_ref()
            .ok_or_else(|| anyhow!("failed to initialize SSH client"))
    }
}

impl SshDirectConnector for ProxySshConnector<'_> {
    fn direct_tcpip_stream<'a>(
        &'a mut self,
        remote_tcp: SocketAddr,
    ) -> RemoteHelperFuture<'a, Result<BoxedRemoteStream>> {
        Box::pin(async move {
            let client = self.client().await?;
            let stream = client
                .direct_tcpip_stream(remote_tcp.ip().to_string(), remote_tcp.port())
                .await?;
            Ok(Box::new(stream) as BoxedRemoteStream)
        })
    }

    fn exec_helper_stream<'a>(
        &'a mut self,
        _intent: &'a RemoteHelperOpenIntent,
    ) -> RemoteHelperFuture<'a, Result<BoxedRemoteStream>> {
        Box::pin(async move {
            let args = self.args;
            let client = self.client().await?;
            let remote_path = ensure_helper(args, client, HelperCapability::Stdio).await?;
            let remote_os = match args.remote_os {
                cli::RemoteOs::Auto => cli::RemoteOs::Unix,
                other => other,
            };
            let command = remote_stdio_command(&remote_path, remote_os);
            Ok(Box::new(client.exec_stream(command).await?) as BoxedRemoteStream)
        })
    }
}

fn local_node_name() -> String {
    format!(
        "{}@{}",
        whoami::username().unwrap_or_else(|_| "unknown".to_string()),
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    )
}

pub async fn open_remote_reverse_socks(
    args: &cli::ProxyArgs,
    remote_listen: SocketAddr,
) -> Result<ssh_client::SshStream> {
    let client = ssh_client::Client::connect_proxy_args(args).await?;
    let remote_path = ensure_helper(
        args,
        &client,
        HelperCapability::ReverseSocks {
            listen: remote_listen,
        },
    )
    .await?;
    let remote_os = match args.remote_os {
        cli::RemoteOs::Auto => cli::RemoteOs::Unix,
        other => other,
    };
    let command = remote_reverse_socks_command(&remote_path, remote_os, remote_listen);
    client.exec_stream(command).await
}
