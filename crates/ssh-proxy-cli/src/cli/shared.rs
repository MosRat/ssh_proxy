use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use ssh_proxy_core::{intent, model};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpTarget {
    pub host: String,
    pub port: u16,
}

impl std::fmt::Display for TcpTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl FromStr for TcpTarget {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = value.strip_prefix('[') {
            let Some((host, tail)) = rest.split_once("]:") else {
                return Err("expected [ipv6]:port".to_string());
            };
            let port = tail
                .parse::<u16>()
                .map_err(|_| format!("invalid TCP target port {tail:?}"))?;
            return Ok(Self {
                host: host.to_string(),
                port,
            });
        }
        let Some((host, port)) = value.rsplit_once(':') else {
            return Err("expected host:port".to_string());
        };
        if host.is_empty() {
            return Err("TCP target host cannot be empty".to_string());
        }
        let port = port
            .parse::<u16>()
            .map_err(|_| format!("invalid TCP target port {port:?}"))?;
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
}

impl From<TcpTarget> for model::TcpTarget {
    fn from(value: TcpTarget) -> Self {
        Self {
            host: value.host,
            port: value.port,
        }
    }
}

impl From<model::TcpTarget> for TcpTarget {
    fn from(value: model::TcpTarget) -> Self {
        Self {
            host: value.host,
            port: value.port,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DeployMode {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Always")]
    Always,
    #[serde(alias = "Never")]
    Never,
}

impl From<DeployMode> for intent::DeploymentPolicy {
    fn from(value: DeployMode) -> Self {
        match value {
            DeployMode::Auto => Self::Auto,
            DeployMode::Always => Self::Always,
            DeployMode::Never => Self::Never,
        }
    }
}

impl From<intent::DeploymentPolicy> for DeployMode {
    fn from(value: intent::DeploymentPolicy) -> Self {
        match value {
            intent::DeploymentPolicy::Auto => Self::Auto,
            intent::DeploymentPolicy::Always => Self::Always,
            intent::DeploymentPolicy::Never => Self::Never,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteOs {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Unix")]
    Unix,
    #[serde(alias = "Windows")]
    Windows,
}

impl From<RemoteOs> for model::RemotePlatform {
    fn from(value: RemoteOs) -> Self {
        match value {
            RemoteOs::Auto => Self::Auto,
            RemoteOs::Unix => Self::Unix,
            RemoteOs::Windows => Self::Windows,
        }
    }
}

impl From<model::RemotePlatform> for RemoteOs {
    fn from(value: model::RemotePlatform) -> Self {
        match value {
            model::RemotePlatform::Auto => Self::Auto,
            model::RemotePlatform::Unix => Self::Unix,
            model::RemotePlatform::Windows => Self::Windows,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemoteTransport {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "SshNative", alias = "ssh_native", alias = "ssh-native")]
    SshNative,
    #[serde(alias = "QuicNative", alias = "quic_native", alias = "native-quic")]
    QuicNative,
    #[serde(alias = "Quic")]
    Quic,
    #[serde(alias = "TlsTcp", alias = "tls_tcp", alias = "tls")]
    TlsTcp,
    #[serde(
        alias = "PlainTcp",
        alias = "DirectTcp",
        alias = "plain_tcp",
        alias = "direct_tcp"
    )]
    PlainTcp,
    #[serde(alias = "Exec")]
    Exec,
    #[serde(alias = "Tcp")]
    Tcp,
}

impl From<RemoteTransport> for model::TransportMode {
    fn from(value: RemoteTransport) -> Self {
        match value {
            RemoteTransport::Auto => Self::Auto,
            RemoteTransport::SshNative => Self::SshNative,
            RemoteTransport::QuicNative => Self::QuicNative,
            RemoteTransport::Quic => Self::Quic,
            RemoteTransport::TlsTcp => Self::TlsTcp,
            RemoteTransport::PlainTcp => Self::PlainTcp,
            RemoteTransport::Exec => Self::Exec,
            RemoteTransport::Tcp => Self::Tcp,
        }
    }
}

impl From<model::TransportMode> for RemoteTransport {
    fn from(value: model::TransportMode) -> Self {
        match value {
            model::TransportMode::Auto => Self::Auto,
            model::TransportMode::SshNative => Self::SshNative,
            model::TransportMode::QuicNative => Self::QuicNative,
            model::TransportMode::Quic => Self::Quic,
            model::TransportMode::TlsTcp => Self::TlsTcp,
            model::TransportMode::PlainTcp => Self::PlainTcp,
            model::TransportMode::Exec => Self::Exec,
            model::TransportMode::Tcp => Self::Tcp,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum PersistMode {
    None,
    Auto,
    Systemd,
    Nohup,
    Launchd,
    Schtasks,
}

impl From<PersistMode> for model::PersistenceMode {
    fn from(value: PersistMode) -> Self {
        match value {
            PersistMode::None => Self::None,
            PersistMode::Auto => Self::Auto,
            PersistMode::Systemd => Self::Systemd,
            PersistMode::Nohup => Self::Nohup,
            PersistMode::Launchd => Self::Launchd,
            PersistMode::Schtasks => Self::Schtasks,
        }
    }
}

impl From<model::PersistenceMode> for PersistMode {
    fn from(value: model::PersistenceMode) -> Self {
        match value {
            model::PersistenceMode::None => Self::None,
            model::PersistenceMode::Auto => Self::Auto,
            model::PersistenceMode::Systemd => Self::Systemd,
            model::PersistenceMode::Nohup => Self::Nohup,
            model::PersistenceMode::Launchd => Self::Launchd,
            model::PersistenceMode::Schtasks => Self::Schtasks,
        }
    }
}
