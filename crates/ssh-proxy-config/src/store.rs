use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

use crate::{
    io::save_text_file_private,
    paths::{config_path, routes_path},
    schema::{AppConfig, CONFIG_SCHEMA_VERSION, NodeIdentity, TokenMetadata},
};

impl AppConfig {
    pub fn load_default() -> Result<Self> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: Self =
            toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
        config.validate_schema()?;
        Ok(config)
    }

    pub fn save_default(&self) -> Result<()> {
        self.validate_schema()?;
        let path = config_path()?;
        let text = toml::to_string_pretty(self).context("failed to encode config TOML")?;
        save_text_file_private(&path, &text)
    }

    pub fn validate_schema(&self) -> Result<()> {
        if self.schema_version > CONFIG_SCHEMA_VERSION {
            bail!(
                "config schema_version {} is newer than this binary supports ({CONFIG_SCHEMA_VERSION}); upgrade ssh_proxy",
                self.schema_version
            );
        }
        Ok(())
    }

    pub fn ensure_daemon_token(&mut self) -> Result<String> {
        if let Some(token) = &self.daemon.token {
            if self.daemon.token_metadata.is_none() {
                self.daemon.token_metadata = Some(TokenMetadata::new("daemon-control-transport"));
            }
            return Ok(token.clone());
        }
        let token = generate_token()?;
        self.daemon.token = Some(token.clone());
        self.daemon.token_metadata = Some(TokenMetadata::new("daemon-control-transport"));
        Ok(token)
    }

    pub fn rotate_daemon_token(&mut self) -> Result<String> {
        let token = generate_token()?;
        let generation = self
            .daemon
            .token_metadata
            .as_ref()
            .map(|metadata| metadata.generation.saturating_add(1))
            .unwrap_or(1);
        self.daemon.token = Some(token.clone());
        self.daemon.token_metadata = Some(TokenMetadata::rotated(
            "daemon-control-transport",
            generation,
        ));
        Ok(token)
    }

    pub fn ensure_node_identity(&mut self) -> Result<NodeIdentity> {
        if self.identity.node_id.is_none() {
            self.identity.node_id = Some(format!("spx-{}", generate_token()?));
        }
        if self.identity.node_name.is_none() {
            self.identity.node_name = Some(default_node_name());
        }
        if self.identity.secret.is_none() {
            self.identity.secret = Some(generate_token()?);
        }
        Ok(self.identity.clone())
    }

    pub fn default_route_store_path(&self) -> Result<std::path::PathBuf> {
        Ok(self
            .daemon
            .routes_path
            .as_ref()
            .map(expand_path)
            .unwrap_or(routes_path()?))
    }
}

pub fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("failed to generate secure transport token")?;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write to String cannot fail");
    }
    Ok(out)
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn default_node_name() -> String {
    let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{user}@{host}")
}

pub fn is_addr_available(addr: SocketAddr) -> bool {
    std::net::TcpListener::bind(addr).is_ok()
}

pub fn first_available_addr(preferred: SocketAddr, span: u16) -> SocketAddr {
    if preferred.port() != 0 && is_addr_available(preferred) {
        return preferred;
    }
    let start = if preferred.port() == 0 {
        19080
    } else {
        preferred.port()
    };
    let ip = preferred.ip();
    for offset in 0..span {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        let candidate = SocketAddr::new(ip, port);
        if is_addr_available(candidate) {
            return candidate;
        }
    }
    preferred
}

pub fn expand_path(path: &std::path::PathBuf) -> std::path::PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    path.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ProxyProfile;

    #[test]
    fn config_schema_round_trips_core_profile_fields() {
        let config = AppConfig {
            defaults: ProxyProfile {
                tcp_target: Some("example.com:443".parse().unwrap()),
                workload_hint: Some(ssh_proxy_core::model::WorkloadHint::Mixed),
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&config).unwrap();
        let parsed = toml::from_str::<AppConfig>(&text).unwrap();
        assert_eq!(
            parsed.defaults.tcp_target.as_ref().unwrap().host,
            "example.com"
        );
        assert_eq!(
            parsed.defaults.workload_hint,
            Some(ssh_proxy_core::model::WorkloadHint::Mixed)
        );
    }
}
