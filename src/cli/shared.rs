use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum PersistMode {
    None,
    Auto,
    Systemd,
    Nohup,
    Launchd,
    Schtasks,
}
