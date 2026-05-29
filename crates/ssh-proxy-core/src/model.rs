use std::str::FromStr;

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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RemotePlatform {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Unix")]
    Unix,
    #[serde(alias = "Windows")]
    Windows,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TransportMode {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PersistenceMode {
    None,
    Auto,
    Systemd,
    Nohup,
    Launchd,
    Schtasks,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RouteDirection {
    LocalUsesRemote,
    RemoteUsesLocal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RouteConnectMode {
    Auto,
    Direct,
    ReverseLink,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadHint {
    Large,
    Concurrent,
    Mixed,
}

impl std::fmt::Display for RemotePlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Unix => "unix",
            Self::Windows => "windows",
        })
    }
}

impl std::fmt::Display for TransportMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::SshNative => "ssh-native",
            Self::QuicNative => "quic-native",
            Self::Quic => "quic",
            Self::TlsTcp => "tls-tcp",
            Self::PlainTcp => "plain-tcp",
            Self::Exec => "exec",
            Self::Tcp => "tcp",
        })
    }
}

impl std::fmt::Display for PersistenceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::Auto => "auto",
            Self::Systemd => "systemd",
            Self::Nohup => "nohup",
            Self::Launchd => "launchd",
            Self::Schtasks => "schtasks",
        })
    }
}

impl std::fmt::Display for RouteDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LocalUsesRemote => "local-uses-remote",
            Self::RemoteUsesLocal => "remote-uses-local",
        })
    }
}

impl std::fmt::Display for RouteConnectMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Direct => "direct",
            Self::ReverseLink => "reverse-link",
        })
    }
}

impl std::fmt::Display for WorkloadHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Large => "large",
            Self::Concurrent => "concurrent",
            Self::Mixed => "mixed",
        })
    }
}

macro_rules! impl_from_str {
    ($ty:ty, $parse:expr) => {
        impl FromStr for $ty {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                $parse(value).ok_or_else(|| format!("unknown {} value {value:?}", stringify!($ty)))
            }
        }
    };
}

impl_from_str!(RemotePlatform, |value: &str| {
    match normalize(value).as_str() {
        "auto" => Some(RemotePlatform::Auto),
        "unix" | "linux" | "macos" => Some(RemotePlatform::Unix),
        "windows" => Some(RemotePlatform::Windows),
        _ => None,
    }
});

impl_from_str!(TransportMode, |value: &str| {
    match normalize(value).as_str() {
        "auto" => Some(TransportMode::Auto),
        "ssh-native" => Some(TransportMode::SshNative),
        "quic-native" | "native-quic" => Some(TransportMode::QuicNative),
        "quic" => Some(TransportMode::Quic),
        "tls-tcp" | "tls" => Some(TransportMode::TlsTcp),
        "plain-tcp" | "direct-tcp" => Some(TransportMode::PlainTcp),
        "exec" => Some(TransportMode::Exec),
        "tcp" => Some(TransportMode::Tcp),
        _ => None,
    }
});

impl_from_str!(PersistenceMode, |value: &str| {
    match normalize(value).as_str() {
        "none" => Some(PersistenceMode::None),
        "auto" => Some(PersistenceMode::Auto),
        "systemd" => Some(PersistenceMode::Systemd),
        "nohup" => Some(PersistenceMode::Nohup),
        "launchd" => Some(PersistenceMode::Launchd),
        "schtasks" => Some(PersistenceMode::Schtasks),
        _ => None,
    }
});

impl_from_str!(RouteDirection, |value: &str| {
    match normalize(value).as_str() {
        "local-uses-remote" => Some(RouteDirection::LocalUsesRemote),
        "remote-uses-local" => Some(RouteDirection::RemoteUsesLocal),
        _ => None,
    }
});

impl_from_str!(RouteConnectMode, |value: &str| {
    match normalize(value).as_str() {
        "auto" => Some(RouteConnectMode::Auto),
        "direct" => Some(RouteConnectMode::Direct),
        "reverse-link" => Some(RouteConnectMode::ReverseLink),
        _ => None,
    }
});

impl_from_str!(WorkloadHint, |value: &str| {
    match normalize(value).as_str() {
        "large" => Some(WorkloadHint::Large),
        "concurrent" => Some(WorkloadHint::Concurrent),
        "mixed" => Some(WorkloadHint::Mixed),
        _ => None,
    }
});

fn normalize(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    for (idx, ch) in value.chars().enumerate() {
        if ch == '_' {
            normalized.push('-');
        } else if ch.is_ascii_uppercase() {
            if idx > 0 {
                normalized.push('-');
            }
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(ch.to_ascii_lowercase());
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcp_target_preserves_existing_parser_shape() {
        assert_eq!(
            "example.com:443".parse::<TcpTarget>().unwrap(),
            TcpTarget {
                host: "example.com".to_string(),
                port: 443
            }
        );
        assert_eq!(
            "[::1]:8443".parse::<TcpTarget>().unwrap(),
            TcpTarget {
                host: "::1".to_string(),
                port: 8443
            }
        );
    }

    #[test]
    fn command_neutral_modes_accept_legacy_aliases() {
        assert_eq!("ssh_native".parse(), Ok(TransportMode::SshNative));
        assert_eq!("TlsTcp".parse(), Ok(TransportMode::TlsTcp));
        assert_eq!("linux".parse(), Ok(RemotePlatform::Unix));
        assert_eq!("reverse_link".parse(), Ok(RouteConnectMode::ReverseLink));
    }

    #[test]
    fn model_values_serialize_as_kebab_case() {
        assert_eq!(
            serde_json::to_string(&TransportMode::PlainTcp).unwrap(),
            "\"plain-tcp\""
        );
        assert_eq!(
            serde_json::to_string(&RouteDirection::RemoteUsesLocal).unwrap(),
            "\"remote-uses-local\""
        );
    }
}
