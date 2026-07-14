use anyhow::Result;
use bytes::Bytes;
use tokio::sync::mpsc;

use crate::{bridge, protocol::UdpDatagram};

#[derive(Debug, Clone)]
pub struct TcpTarget {
    pub host: String,
    pub port: u16,
    pub egress_proxy: Option<String>,
}

impl TcpTarget {
    pub fn new(host: String, port: u16, egress_proxy: Option<String>) -> Self {
        Self {
            host,
            port,
            egress_proxy,
        }
    }
}

#[derive(Clone)]
pub struct SpxRouteLink {
    bridge: bridge::BridgeHandle,
}

impl SpxRouteLink {
    pub fn new(bridge: bridge::BridgeHandle) -> Self {
        Self { bridge }
    }

    pub async fn open_tcp(&self, target: TcpTarget) -> Result<SpxTcpFlow> {
        let (id, rx) = self
            .bridge
            .open_tcp(target.host, target.port, target.egress_proxy)
            .await?;
        Ok(SpxTcpFlow {
            id,
            link: self.clone(),
            rx,
        })
    }

    async fn send_tcp(&self, id: u32, data: Bytes) -> Result<()> {
        self.bridge.send_data(id, data).await
    }

    async fn close_tcp(&self, id: u32, reason: impl Into<String>) {
        self.bridge.close(id, reason).await;
    }

    pub async fn register_udp(&self) -> SpxUdpAssociation {
        let (id, rx) = self.bridge.register_udp().await;
        SpxUdpAssociation {
            id,
            link: self.clone(),
            rx,
        }
    }

    async fn send_udp(&self, id: u32, datagram: UdpDatagram) -> Result<()> {
        self.bridge.send_udp(id, datagram).await
    }

    async fn unregister_udp(&self, id: u32) {
        self.bridge.unregister_udp(id).await;
    }
}

pub struct SpxTcpFlow {
    id: u32,
    link: SpxRouteLink,
    rx: mpsc::Receiver<Bytes>,
}

impl SpxTcpFlow {
    pub fn split(self) -> (SpxTcpSender, SpxTcpReceiver, SpxTcpCloser) {
        let sender = SpxTcpSender {
            id: self.id,
            link: self.link.clone(),
        };
        let closer = SpxTcpCloser {
            id: self.id,
            link: self.link,
        };
        (sender, SpxTcpReceiver { rx: self.rx }, closer)
    }
}

#[derive(Clone)]
pub struct SpxTcpSender {
    id: u32,
    link: SpxRouteLink,
}

impl SpxTcpSender {
    pub async fn send(&self, data: Bytes) -> Result<()> {
        self.link.send_tcp(self.id, data).await
    }
}

pub struct SpxTcpReceiver {
    rx: mpsc::Receiver<Bytes>,
}

impl SpxTcpReceiver {
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.rx.recv().await
    }
}

pub struct SpxTcpCloser {
    id: u32,
    link: SpxRouteLink,
}

impl SpxTcpCloser {
    pub async fn close(self, reason: impl Into<String>) {
        self.link.close_tcp(self.id, reason).await;
    }
}

pub struct SpxUdpAssociation {
    id: u32,
    link: SpxRouteLink,
    rx: mpsc::Receiver<UdpDatagram>,
}

impl SpxUdpAssociation {
    pub fn split(self) -> (SpxUdpSender, SpxUdpReceiver, SpxUdpCloser) {
        let sender = SpxUdpSender {
            id: self.id,
            link: self.link.clone(),
        };
        let closer = SpxUdpCloser {
            id: self.id,
            link: self.link,
        };
        (sender, SpxUdpReceiver { rx: self.rx }, closer)
    }
}

#[derive(Clone)]
pub struct SpxUdpSender {
    id: u32,
    link: SpxRouteLink,
}

impl SpxUdpSender {
    pub async fn send(&self, datagram: UdpDatagram) -> Result<()> {
        self.link.send_udp(self.id, datagram).await
    }
}

pub struct SpxUdpReceiver {
    rx: mpsc::Receiver<UdpDatagram>,
}

impl SpxUdpReceiver {
    pub async fn recv(&mut self) -> Option<UdpDatagram> {
        self.rx.recv().await
    }
}

pub struct SpxUdpCloser {
    id: u32,
    link: SpxRouteLink,
}

impl SpxUdpCloser {
    pub async fn close(self) {
        self.link.unregister_udp(self.id).await;
    }
}
