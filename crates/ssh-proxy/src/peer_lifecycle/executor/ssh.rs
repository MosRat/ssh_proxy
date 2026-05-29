use std::net::SocketAddr;

use anyhow::{Context, bail};

use crate::{ssh_client, ssh_client::ExecOutput};

use super::{BoxExecutorFuture, PeerExecutor};

pub(crate) struct SshExecutor<'a> {
    client: &'a ssh_client::Client,
}

impl<'a> SshExecutor<'a> {
    pub(crate) fn new(client: &'a ssh_client::Client) -> Self {
        Self { client }
    }
}

impl PeerExecutor for SshExecutor<'_> {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move { self.client.exec_capture(command, stdin).await })
    }

    fn upload_bytes<'a>(&'a self, command: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move { self.client.exec_upload(command, bytes).await })
    }

    fn read_artifact<'a>(&'a self, command: String) -> BoxExecutorFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let output = self
                .client
                .exec_capture(command.clone(), None)
                .await
                .with_context(|| format!("failed to read remote lifecycle artifact: {command}"))?;
            if output.exit_status != 0 {
                bail!(
                    "remote lifecycle artifact read failed with status {}: {}",
                    output.exit_status,
                    output.stderr.trim()
                );
            }
            Ok(output.stdout.into_bytes())
        })
    }

    fn probe_tcp<'a>(&'a self, addr: SocketAddr) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            let _stream = self
                .client
                .direct_tcpip_stream(addr.ip().to_string(), addr.port())
                .await
                .with_context(|| format!("failed to probe remote TCP endpoint {addr}"))?;
            Ok(())
        })
    }
}
