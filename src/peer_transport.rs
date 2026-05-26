use std::{
    fmt, fs::File, future::Future, io::BufReader, net::SocketAddr, path::Path, str::FromStr,
    sync::Arc, time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time,
};
use tokio_rustls::rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
    server::WebPkiClientVerifier,
};

const HANDSHAKE_MAGIC: &[u8; 4] = b"SPX1";
const MAX_HANDSHAKE: usize = 64 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
pub const PEER_VERSION: u16 = 1;
pub const QUIC_MAX_BIDI_STREAMS: u32 = 256;
pub const QUIC_STREAM_RECEIVE_WINDOW: u32 = 2 * 1024 * 1024;
pub const QUIC_RECEIVE_WINDOW: u32 = 16 * 1024 * 1024;
pub const QUIC_KEEP_ALIVE_INTERVAL_SECS: u64 = 10;
pub const QUIC_IDLE_TIMEOUT_SECS: u64 = 60;
pub const QUIC_UDP_RUNTIME: &str = "quinn-udp";
pub const QUIC_PACKETIZATION: &str = "quinn-managed-udp";
pub const QUIC_UDP_GSO_SOURCE: &str =
    "unknown: quinn 0.11 endpoint API does not expose effective UDP GSO capability";
const MAX_QUIC_MAX_BIDI_STREAMS: u32 = 4096;
const MAX_QUIC_STREAM_RECEIVE_WINDOW: u32 = 64 * 1024 * 1024;
const MAX_QUIC_RECEIVE_WINDOW: u32 = 256 * 1024 * 1024;
const MAX_QUIC_KEEP_ALIVE_INTERVAL_SECS: u64 = 300;
const MAX_QUIC_IDLE_TIMEOUT_SECS: u64 = 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicTransportOptions {
    pub max_bidi_streams: u32,
    pub stream_receive_window: u32,
    pub receive_window: u32,
    pub keep_alive_interval_secs: u64,
    pub idle_timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuicRuntimeDiagnostics {
    pub platform_os: &'static str,
    pub platform_arch: &'static str,
    pub udp_runtime: &'static str,
    pub udp_gso: Option<bool>,
    pub udp_gso_source: &'static str,
    pub packetization: &'static str,
    pub transport_options: QuicTransportOptions,
}

impl Default for QuicTransportOptions {
    fn default() -> Self {
        Self {
            max_bidi_streams: QUIC_MAX_BIDI_STREAMS,
            stream_receive_window: QUIC_STREAM_RECEIVE_WINDOW,
            receive_window: QUIC_RECEIVE_WINDOW,
            keep_alive_interval_secs: QUIC_KEEP_ALIVE_INTERVAL_SECS,
            idle_timeout_secs: QUIC_IDLE_TIMEOUT_SECS,
        }
    }
}

pub fn default_quic_max_bidi_streams() -> u32 {
    QUIC_MAX_BIDI_STREAMS
}

pub fn default_quic_stream_receive_window() -> u32 {
    QUIC_STREAM_RECEIVE_WINDOW
}

pub fn default_quic_receive_window() -> u32 {
    QUIC_RECEIVE_WINDOW
}

pub fn default_quic_keep_alive_interval_secs() -> u64 {
    QUIC_KEEP_ALIVE_INTERVAL_SECS
}

pub fn default_quic_idle_timeout_secs() -> u64 {
    QUIC_IDLE_TIMEOUT_SECS
}

impl QuicTransportOptions {
    pub fn new(
        max_bidi_streams: u32,
        stream_receive_window: u32,
        receive_window: u32,
        keep_alive_interval_secs: u64,
        idle_timeout_secs: u64,
    ) -> Result<Self> {
        let options = Self {
            max_bidi_streams,
            stream_receive_window,
            receive_window,
            keep_alive_interval_secs,
            idle_timeout_secs,
        };
        options.validate()
    }

    pub fn validate(self) -> Result<Self> {
        if self.max_bidi_streams == 0 {
            bail!("quic_max_bidi_streams must be greater than zero");
        }
        if self.stream_receive_window == 0 {
            bail!("quic_stream_receive_window must be greater than zero");
        }
        if self.receive_window == 0 {
            bail!("quic_receive_window must be greater than zero");
        }
        if self.keep_alive_interval_secs == 0 {
            bail!("quic_keep_alive_interval_secs must be greater than zero");
        }
        if self.idle_timeout_secs == 0 {
            bail!("quic_idle_timeout_secs must be greater than zero");
        }
        Ok(Self {
            max_bidi_streams: self.max_bidi_streams.min(MAX_QUIC_MAX_BIDI_STREAMS),
            stream_receive_window: self
                .stream_receive_window
                .min(MAX_QUIC_STREAM_RECEIVE_WINDOW),
            receive_window: self.receive_window.min(MAX_QUIC_RECEIVE_WINDOW),
            keep_alive_interval_secs: self
                .keep_alive_interval_secs
                .min(MAX_QUIC_KEEP_ALIVE_INTERVAL_SECS),
            idle_timeout_secs: self.idle_timeout_secs.min(MAX_QUIC_IDLE_TIMEOUT_SECS),
        })
    }

    pub fn keep_alive_interval(self) -> Duration {
        Duration::from_secs(self.keep_alive_interval_secs)
    }

    pub fn idle_timeout(self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }
}

pub fn quic_runtime_diagnostics(options: QuicTransportOptions) -> QuicRuntimeDiagnostics {
    QuicRuntimeDiagnostics {
        platform_os: std::env::consts::OS,
        platform_arch: std::env::consts::ARCH,
        udp_runtime: QUIC_UDP_RUNTIME,
        udp_gso: None,
        udp_gso_source: QUIC_UDP_GSO_SOURCE,
        packetization: QUIC_PACKETIZATION,
        transport_options: options.validate().unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeerProtocol {
    SshNative,
    QuicNative,
    Quic,
    TlsTcp,
    Tcp,
    SshDirect,
    SshExec,
}

impl fmt::Display for PeerProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::SshNative => "ssh-native",
            Self::QuicNative => "quic-native",
            Self::Quic => "quic",
            Self::TlsTcp => "tls-tcp",
            Self::Tcp => "tcp",
            Self::SshDirect => "ssh-direct",
            Self::SshExec => "ssh-exec",
        };
        f.write_str(value)
    }
}

impl PeerProtocol {
    pub fn data_plane_label(self) -> &'static str {
        match self {
            Self::SshNative => "ssh-native",
            Self::QuicNative => "quic-native",
            Self::Quic => "quic-framed",
            Self::TlsTcp => "tls-spx-framed",
            Self::Tcp => "plain-spx-framed",
            Self::SshDirect => "ssh-direct-spx",
            Self::SshExec => "ssh-exec-spx",
        }
    }
}

impl FromStr for PeerProtocol {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "quic" => Ok(Self::Quic),
            "quic-native" | "quic_native" | "native-quic" | "native_quic" => Ok(Self::QuicNative),
            "ssh-native" | "ssh_native" | "native-ssh" | "native_ssh" => Ok(Self::SshNative),
            "tls-tcp" | "tls_tcp" | "tls" => Ok(Self::TlsTcp),
            "tcp" | "plain-tcp" | "plain_tcp" | "direct-tcp" | "direct_tcp" => Ok(Self::Tcp),
            "ssh-direct" | "ssh_direct" | "ssh-tcp" | "ssh_tcp" => Ok(Self::SshDirect),
            "ssh-exec" | "ssh_exec" | "exec" => Ok(Self::SshExec),
            _ => bail!("invalid peer protocol {value:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEndpoint {
    pub protocol: PeerProtocol,
    pub addr: Option<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHello {
    pub version: u16,
    pub node: String,
    pub protocols: Vec<PeerProtocol>,
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub feature_bits: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerWelcome {
    pub version: u16,
    pub node: String,
    pub accepted: Option<PeerProtocol>,
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub feature_bits: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkHints {
    pub peer_addr: Option<SocketAddr>,
    pub ssh_available: bool,
    pub allow_plain_tcp: bool,
    pub prefer_low_latency: bool,
}

impl Default for NetworkHints {
    fn default() -> Self {
        Self {
            peer_addr: None,
            ssh_available: true,
            allow_plain_tcp: false,
            prefer_low_latency: true,
        }
    }
}

pub fn auto_candidates(hints: &NetworkHints) -> Vec<PeerEndpoint> {
    let mut candidates = Vec::new();

    if let Some(addr) = hints.peer_addr {
        if hints.prefer_low_latency {
            candidates.push(PeerEndpoint {
                protocol: PeerProtocol::Quic,
                addr: Some(addr),
            });
        }
        candidates.push(PeerEndpoint {
            protocol: PeerProtocol::TlsTcp,
            addr: Some(addr),
        });
        if hints.allow_plain_tcp {
            candidates.push(PeerEndpoint {
                protocol: PeerProtocol::Tcp,
                addr: Some(addr),
            });
        }
    }

    if hints.ssh_available {
        candidates.push(PeerEndpoint {
            protocol: PeerProtocol::SshDirect,
            addr: hints.peer_addr,
        });
        candidates.push(PeerEndpoint {
            protocol: PeerProtocol::SshExec,
            addr: None,
        });
    }

    candidates
}

pub fn implemented_auto_candidates(hints: &NetworkHints) -> Vec<PeerEndpoint> {
    auto_candidates(hints)
        .into_iter()
        .filter(|candidate| {
            matches!(
                candidate.protocol,
                PeerProtocol::Quic
                    | PeerProtocol::TlsTcp
                    | PeerProtocol::Tcp
                    | PeerProtocol::SshDirect
                    | PeerProtocol::SshExec
            )
        })
        .collect()
}

pub fn default_features() -> Vec<String> {
    [
        "frames-v1",
        "socks5h",
        "tcp-connect",
        "udp-associate",
        "ssh-native-direct-tcpip",
        "quic-native-streams-v1",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}

pub fn default_feature_bits() -> Map<String, Value> {
    default_features()
        .into_iter()
        .map(|feature| (feature, Value::Bool(true)))
        .collect()
}

pub fn select_supported_protocol(
    requested: &[PeerProtocol],
    supported: &[PeerProtocol],
) -> Option<PeerProtocol> {
    requested
        .iter()
        .copied()
        .find(|protocol| supported.contains(protocol))
}

#[allow(dead_code)]
pub fn tls_server_config(
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ServerConfig>> {
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("failed to build TLS server config")?;
    Ok(Arc::new(config))
}

pub fn tls_server_config_with_client_auth(
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    client_roots: Vec<CertificateDer<'static>>,
) -> Result<Arc<ServerConfig>> {
    let mut store = RootCertStore::empty();
    for cert in client_roots {
        store
            .add(cert)
            .context("failed to add TLS client root certificate")?;
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(store))
        .build()
        .context("failed to build TLS client certificate verifier")?;
    let config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(cert_chain, key)
        .context("failed to build mTLS server config")?;
    Ok(Arc::new(config))
}

#[allow(dead_code)]
pub fn tls_client_config(roots: Vec<CertificateDer<'static>>) -> Result<Arc<ClientConfig>> {
    let mut store = RootCertStore::empty();
    for cert in roots {
        store
            .add(cert)
            .context("failed to add TLS root certificate")?;
    }
    let config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

pub fn tls_client_config_with_client_auth(
    roots: Vec<CertificateDer<'static>>,
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ClientConfig>> {
    let mut store = RootCertStore::empty();
    for cert in roots {
        store
            .add(cert)
            .context("failed to add TLS root certificate")?;
    }
    let config = ClientConfig::builder()
        .with_root_certificates(store)
        .with_client_auth_cert(cert_chain, key)
        .context("failed to build TLS client auth config")?;
    Ok(Arc::new(config))
}

pub fn quic_server_config(
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    options: QuicTransportOptions,
) -> Result<quinn::ServerConfig> {
    let mut config = quinn::ServerConfig::with_single_cert(cert_chain, key)
        .context("failed to build QUIC server config")?;
    config.transport_config(quic_transport_config(options)?);
    Ok(config)
}

pub fn quic_client_config(
    roots: Vec<CertificateDer<'static>>,
    options: QuicTransportOptions,
) -> Result<quinn::ClientConfig> {
    let mut store = RootCertStore::empty();
    for cert in roots {
        store
            .add(cert)
            .context("failed to add QUIC root certificate")?;
    }
    let mut config = quinn::ClientConfig::with_root_certificates(Arc::new(store))
        .context("failed to build QUIC client config")?;
    config.transport_config(quic_transport_config(options)?);
    Ok(config)
}

fn quic_transport_config(options: QuicTransportOptions) -> Result<Arc<quinn::TransportConfig>> {
    let options = options.validate()?;
    let mut transport = quinn::TransportConfig::default();
    transport
        .max_concurrent_bidi_streams(quinn::VarInt::from_u32(options.max_bidi_streams))
        .stream_receive_window(quinn::VarInt::from_u32(options.stream_receive_window))
        .receive_window(quinn::VarInt::from_u32(options.receive_window))
        .send_fairness(true)
        .keep_alive_interval(Some(options.keep_alive_interval()))
        .max_idle_timeout(Some(options.idle_timeout().try_into()?));
    Ok(Arc::new(transport))
}

pub fn load_cert_chain(path: &Path) -> Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path)
        .with_context(|| format!("failed to open certificate PEM {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("failed to read certificate PEM {}", path.display()))?;
    if certs.is_empty() {
        bail!(
            "certificate PEM {} contained no certificates",
            path.display()
        );
    }
    Ok(certs)
}

pub fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>> {
    let file = File::open(path)
        .with_context(|| format!("failed to open private key PEM {}", path.display()))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .with_context(|| format!("failed to read private key PEM {}", path.display()))?
        .ok_or_else(|| anyhow::anyhow!("private key PEM {} contained no key", path.display()))
}

pub async fn client_handshake<S>(
    stream: &mut S,
    node: impl Into<String>,
    protocol: PeerProtocol,
) -> Result<PeerWelcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello = PeerHello {
        version: PEER_VERSION,
        node: node.into(),
        protocols: vec![protocol],
        features: default_features(),
        feature_bits: default_feature_bits(),
        binary_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
    };
    with_handshake_timeout(write_handshake(stream, &hello), "write peer hello").await?;
    let welcome: PeerWelcome =
        with_handshake_timeout(read_handshake(stream), "read peer welcome").await?;
    if welcome.version != PEER_VERSION {
        bail!(
            "peer version mismatch: local={}, remote={}",
            PEER_VERSION,
            welcome.version
        );
    }
    match welcome.accepted {
        Some(accepted) if accepted == protocol => Ok(welcome),
        Some(accepted) => bail!("peer accepted unexpected protocol {accepted}"),
        None => bail!("peer rejected handshake: {}", welcome.message),
    }
}

pub async fn server_handshake<S>(
    stream: &mut S,
    node: impl Into<String>,
    supported: &[PeerProtocol],
) -> Result<PeerHello>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello: PeerHello =
        with_handshake_timeout(read_handshake(stream), "read peer hello").await?;
    let accepted = if hello.version == PEER_VERSION {
        select_supported_protocol(&hello.protocols, supported)
    } else {
        None
    };
    let message = if hello.version != PEER_VERSION {
        format!(
            "unsupported peer version {}; expected {}",
            hello.version, PEER_VERSION
        )
    } else if accepted.is_none() {
        "no mutually supported peer protocol".to_string()
    } else {
        "ok".to_string()
    };
    let welcome = PeerWelcome {
        version: PEER_VERSION,
        node: node.into(),
        accepted,
        features: default_features(),
        feature_bits: default_feature_bits(),
        binary_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        os: Some(std::env::consts::OS.to_string()),
        arch: Some(std::env::consts::ARCH.to_string()),
        message,
    };
    with_handshake_timeout(write_handshake(stream, &welcome), "write peer welcome").await?;
    if accepted.is_none() {
        bail!("peer handshake rejected: {}", welcome.message);
    }
    Ok(hello)
}

async fn write_handshake<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).context("failed to encode peer handshake")?;
    if payload.len() > MAX_HANDSHAKE {
        bail!("peer handshake too large: {}", payload.len());
    }
    writer.write_all(HANDSHAKE_MAGIC).await?;
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

async fn with_handshake_timeout<F, T>(operation: F, label: &'static str) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    time::timeout(HANDSHAKE_TIMEOUT, operation)
        .await
        .with_context(|| format!("{label} timed out after {}s", HANDSHAKE_TIMEOUT.as_secs()))?
}

async fn read_handshake<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut magic = [0_u8; 4];
    reader
        .read_exact(&mut magic)
        .await
        .context("failed to read peer handshake magic")?;
    if &magic != HANDSHAKE_MAGIC {
        bail!("invalid peer handshake magic");
    }
    let mut len = [0_u8; 4];
    reader
        .read_exact(&mut len)
        .await
        .context("failed to read peer handshake length")?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_HANDSHAKE {
        bail!("peer handshake too large: {len}");
    }
    let mut payload = vec![0_u8; len];
    reader
        .read_exact(&mut payload)
        .await
        .context("failed to read peer handshake payload")?;
    serde_json::from_slice(&payload).context("failed to decode peer handshake")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn auto_prefers_quic_then_tls_then_ssh_fallback() {
        let hints = NetworkHints {
            peer_addr: Some("127.0.0.1:19080".parse().unwrap()),
            ssh_available: true,
            allow_plain_tcp: false,
            prefer_low_latency: true,
        };
        let protocols = auto_candidates(&hints)
            .into_iter()
            .map(|candidate| candidate.protocol)
            .collect::<Vec<_>>();
        assert_eq!(
            protocols,
            vec![
                PeerProtocol::Quic,
                PeerProtocol::TlsTcp,
                PeerProtocol::SshDirect,
                PeerProtocol::SshExec
            ]
        );
    }

    #[test]
    fn implemented_auto_candidates_match_current_data_plane() {
        let hints = NetworkHints {
            peer_addr: Some("127.0.0.1:19080".parse().unwrap()),
            ssh_available: true,
            allow_plain_tcp: false,
            prefer_low_latency: true,
        };
        let protocols = implemented_auto_candidates(&hints)
            .into_iter()
            .map(|candidate| candidate.protocol)
            .collect::<Vec<_>>();
        assert_eq!(
            protocols,
            vec![
                PeerProtocol::Quic,
                PeerProtocol::TlsTcp,
                PeerProtocol::SshDirect,
                PeerProtocol::SshExec
            ]
        );
    }

    #[test]
    fn implemented_auto_candidates_include_plain_tcp_only_when_allowed() {
        let hints = NetworkHints {
            peer_addr: Some("127.0.0.1:19080".parse().unwrap()),
            ssh_available: true,
            allow_plain_tcp: true,
            prefer_low_latency: true,
        };
        let protocols = implemented_auto_candidates(&hints)
            .into_iter()
            .map(|candidate| candidate.protocol)
            .collect::<Vec<_>>();
        assert_eq!(
            protocols,
            vec![
                PeerProtocol::Quic,
                PeerProtocol::TlsTcp,
                PeerProtocol::Tcp,
                PeerProtocol::SshDirect,
                PeerProtocol::SshExec
            ]
        );
    }

    #[test]
    fn protocol_parser_accepts_operational_aliases() {
        assert_eq!(
            "exec".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::SshExec
        );
        assert_eq!(
            "ssh-tcp".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::SshDirect
        );
        assert_eq!("tls".parse::<PeerProtocol>().unwrap(), PeerProtocol::TlsTcp);
        assert_eq!(
            "direct-tcp".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::Tcp
        );
        assert_eq!(
            "native-quic".parse::<PeerProtocol>().unwrap(),
            PeerProtocol::QuicNative
        );
    }

    #[test]
    fn data_plane_labels_distinguish_framed_and_native_quic() {
        assert_eq!(PeerProtocol::Quic.data_plane_label(), "quic-framed");
        assert_eq!(PeerProtocol::QuicNative.data_plane_label(), "quic-native");
        assert_eq!(PeerProtocol::SshDirect.data_plane_label(), "ssh-direct-spx");
    }

    #[test]
    fn supported_protocol_selection_preserves_client_preference() {
        let requested = [PeerProtocol::QuicNative, PeerProtocol::Quic];
        let supported = [PeerProtocol::Quic, PeerProtocol::QuicNative];

        assert_eq!(
            select_supported_protocol(&requested, &supported),
            Some(PeerProtocol::QuicNative)
        );
        assert_eq!(
            select_supported_protocol(&[PeerProtocol::Quic], &supported),
            Some(PeerProtocol::Quic)
        );
        assert_eq!(
            select_supported_protocol(&[PeerProtocol::SshExec], &supported),
            None
        );
    }

    #[tokio::test]
    async fn client_and_server_handshake_negotiate_protocol() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            server_handshake(
                &mut server,
                "server",
                &[PeerProtocol::SshDirect, PeerProtocol::SshExec],
            )
            .await
        });
        let welcome = client_handshake(&mut client, "client", PeerProtocol::SshDirect)
            .await
            .expect("client handshake");
        let hello = server_task
            .await
            .expect("server task")
            .expect("server handshake");
        assert_eq!(welcome.accepted, Some(PeerProtocol::SshDirect));
        assert_eq!(hello.node, "client");
        assert!(welcome.features.contains(&"frames-v1".to_string()));
    }

    #[tokio::test]
    async fn client_handshake_reports_rejection() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            server_handshake(&mut server, "server", &[PeerProtocol::SshExec]).await
        });
        let err = client_handshake(&mut client, "client", PeerProtocol::Quic)
            .await
            .expect_err("handshake should fail");
        assert!(err.to_string().contains("rejected"));
        let server_err = server_task
            .await
            .expect("server task")
            .expect_err("server rejects");
        assert!(server_err.to_string().contains("rejected"));
    }

    #[test]
    fn quic_transport_options_reject_zero_values() {
        assert!(QuicTransportOptions::new(0, 1, 1, 1, 1).is_err());
        assert!(QuicTransportOptions::new(1, 0, 1, 1, 1).is_err());
        assert!(QuicTransportOptions::new(1, 1, 0, 1, 1).is_err());
        assert!(QuicTransportOptions::new(1, 1, 1, 0, 1).is_err());
        assert!(QuicTransportOptions::new(1, 1, 1, 1, 0).is_err());
    }

    #[test]
    fn quic_transport_options_cap_extreme_values() {
        let options = QuicTransportOptions::new(u32::MAX, u32::MAX, u32::MAX, u64::MAX, u64::MAX)
            .expect("options");

        assert_eq!(options.max_bidi_streams, MAX_QUIC_MAX_BIDI_STREAMS);
        assert_eq!(
            options.stream_receive_window,
            MAX_QUIC_STREAM_RECEIVE_WINDOW
        );
        assert_eq!(options.receive_window, MAX_QUIC_RECEIVE_WINDOW);
        assert_eq!(
            options.keep_alive_interval_secs,
            MAX_QUIC_KEEP_ALIVE_INTERVAL_SECS
        );
        assert_eq!(options.idle_timeout_secs, MAX_QUIC_IDLE_TIMEOUT_SECS);
    }

    #[test]
    fn quic_runtime_diagnostics_are_explicit_when_gso_is_not_exposed() {
        let diagnostics = quic_runtime_diagnostics(QuicTransportOptions::default());

        assert_eq!(diagnostics.udp_runtime, QUIC_UDP_RUNTIME);
        assert_eq!(diagnostics.udp_gso, None);
        assert!(diagnostics.udp_gso_source.contains("unknown"));
        assert_eq!(diagnostics.packetization, QUIC_PACKETIZATION);
        assert_eq!(
            diagnostics.transport_options,
            QuicTransportOptions::default()
        );
    }

    #[tokio::test]
    async fn tls_stream_can_carry_peer_handshake() {
        use tokio_rustls::{
            TlsAcceptor, TlsConnector,
            rustls::pki_types::{PrivatePkcs8KeyDer, ServerName},
        };

        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert = certified.cert.der().clone();
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(
            certified.signing_key.serialize_der(),
        ));
        let server_config = tls_server_config(vec![cert.clone()], key).unwrap();
        let client_config = tls_client_config(vec![cert]).unwrap();
        let acceptor = TlsAcceptor::from(server_config);
        let connector = TlsConnector::from(client_config);
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move {
            let mut stream = acceptor.accept(server_io).await?;
            server_handshake(&mut stream, "server", &[PeerProtocol::TlsTcp]).await
        });

        let server_name = ServerName::try_from("localhost").unwrap().to_owned();
        let mut stream = connector.connect(server_name, client_io).await.unwrap();
        let welcome = client_handshake(&mut stream, "client", PeerProtocol::TlsTcp)
            .await
            .unwrap();
        let hello = server_task.await.unwrap().unwrap();

        assert_eq!(welcome.accepted, Some(PeerProtocol::TlsTcp));
        assert_eq!(welcome.node, "server");
        assert_eq!(hello.node, "client");
    }

    #[tokio::test]
    async fn tls_stream_rejects_wrong_server_name_before_peer_handshake() {
        use tokio_rustls::{
            TlsAcceptor, TlsConnector,
            rustls::pki_types::{PrivatePkcs8KeyDer, ServerName},
        };

        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert = certified.cert.der().clone();
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(
            certified.signing_key.serialize_der(),
        ));
        let server_config = tls_server_config(vec![cert.clone()], key).unwrap();
        let client_config = tls_client_config(vec![cert]).unwrap();
        let acceptor = TlsAcceptor::from(server_config);
        let connector = TlsConnector::from(client_config);
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move { acceptor.accept(server_io).await });

        let server_name = ServerName::try_from("wrong.example").unwrap().to_owned();
        let err = connector
            .connect(server_name, client_io)
            .await
            .expect_err("wrong server name must fail TLS verification");
        let detail = err.to_string();
        assert!(
            detail.contains("certificate") || detail.contains("cert"),
            "{detail}"
        );

        let _ = tokio::time::timeout(Duration::from_secs(1), server_task)
            .await
            .expect("server TLS task should finish after client verification fails");
    }

    #[tokio::test]
    async fn mtls_stream_can_carry_peer_handshake_with_client_certificate() {
        use tokio_rustls::{
            TlsAcceptor, TlsConnector,
            rustls::pki_types::{PrivatePkcs8KeyDer, ServerName},
        };

        let server = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let client = rcgen::generate_simple_self_signed(vec!["client".to_string()]).unwrap();
        let server_cert = server.cert.der().clone();
        let server_key =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(server.signing_key.serialize_der()));
        let client_cert = client.cert.der().clone();
        let client_key =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(client.signing_key.serialize_der()));
        let server_config = tls_server_config_with_client_auth(
            vec![server_cert.clone()],
            server_key,
            vec![client_cert.clone()],
        )
        .unwrap();
        let client_config =
            tls_client_config_with_client_auth(vec![server_cert], vec![client_cert], client_key)
                .unwrap();
        let acceptor = TlsAcceptor::from(server_config);
        let connector = TlsConnector::from(client_config);
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move {
            let mut stream = acceptor.accept(server_io).await?;
            server_handshake(&mut stream, "server", &[PeerProtocol::TlsTcp]).await
        });

        let server_name = ServerName::try_from("localhost").unwrap().to_owned();
        let mut stream = connector.connect(server_name, client_io).await.unwrap();
        let welcome = client_handshake(&mut stream, "client", PeerProtocol::TlsTcp)
            .await
            .unwrap();
        let hello = server_task.await.unwrap().unwrap();

        assert_eq!(welcome.accepted, Some(PeerProtocol::TlsTcp));
        assert_eq!(welcome.node, "server");
        assert_eq!(hello.node, "client");
    }

    #[tokio::test]
    async fn mtls_stream_rejects_missing_client_certificate() {
        use tokio_rustls::{
            TlsAcceptor, TlsConnector,
            rustls::pki_types::{PrivatePkcs8KeyDer, ServerName},
        };

        let server = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let client = rcgen::generate_simple_self_signed(vec!["client".to_string()]).unwrap();
        let server_cert = server.cert.der().clone();
        let server_key =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(server.signing_key.serialize_der()));
        let client_cert = client.cert.der().clone();
        let server_config = tls_server_config_with_client_auth(
            vec![server_cert.clone()],
            server_key,
            vec![client_cert],
        )
        .unwrap();
        let client_config = tls_client_config(vec![server_cert]).unwrap();
        let acceptor = TlsAcceptor::from(server_config);
        let connector = TlsConnector::from(client_config);
        let (client_io, server_io) = tokio::io::duplex(16 * 1024);

        let server_task = tokio::spawn(async move { acceptor.accept(server_io).await });

        let server_name = ServerName::try_from("localhost").unwrap().to_owned();
        let mut stream = connector
            .connect(server_name, client_io)
            .await
            .expect("client sees TLS connect before server auth alert is read");
        let err = client_handshake(&mut stream, "client", PeerProtocol::TlsTcp)
            .await
            .expect_err("server should reject missing client certificate");
        let detail = err.to_string();
        assert!(
            detail.contains("certificate")
                || detail.contains("cert")
                || detail.contains("alert")
                || detail.contains("peer handshake"),
            "{detail}"
        );

        let _ = tokio::time::timeout(Duration::from_secs(1), server_task)
            .await
            .expect("server TLS task should finish after rejecting client");
    }
}
