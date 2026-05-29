use std::{future::Future, net::SocketAddr, pin::Pin};

use anyhow::{Result, bail};

use crate::artifacts::PeerArtifact;
use ssh_proxy_core::command::ExecOutput;

pub type BoxExecutorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceControlAction {
    Install,
    Start,
    Stop,
    Status,
    Rollback,
}

impl ServiceControlAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Status => "status",
            Self::Rollback => "rollback",
        }
    }
}

pub trait PeerExecutor {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput>;

    fn upload_bytes<'a>(&'a self, command: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()>;

    fn write_artifact<'a>(
        &'a self,
        target: String,
        artifact: PeerArtifact,
        bytes: Vec<u8>,
    ) -> BoxExecutorFuture<'a, ()> {
        let _ = artifact;
        self.upload_bytes(target, bytes)
    }

    fn read_artifact<'a>(&'a self, target: String) -> BoxExecutorFuture<'a, Vec<u8>> {
        Box::pin(
            async move { bail!("reading lifecycle artifact is not supported for target {target}") },
        )
    }

    fn stage_binary<'a>(&'a self, source: String, target: String) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            bail!("binary staging from {source} to {target} is not supported by this executor")
        })
    }

    fn probe_tcp<'a>(&'a self, addr: SocketAddr) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move { bail!("TCP probing for {addr} is not supported by this executor") })
    }

    fn service_control<'a>(
        &'a self,
        service_name: String,
        action: ServiceControlAction,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            bail!(
                "service control {} for {service_name} is not supported by this executor",
                action.as_str()
            )
        })
    }
}
