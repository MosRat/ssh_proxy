use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use super::{
    artifacts::{PeerArtifact, PeerArtifactBytes, materialized_peer_artifacts},
    config::PeerConfigFiles,
    report::redact_value,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerStoreRole {
    LocalDaemon,
    RemotePeer,
}

impl PeerStoreRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LocalDaemon => "local_daemon",
            Self::RemotePeer => "remote_peer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PeerStoreLayout {
    pub(crate) role: PeerStoreRole,
    pub(crate) root: String,
}

impl PeerStoreLayout {
    pub(crate) fn new(role: PeerStoreRole, root: impl Into<String>) -> Self {
        Self {
            role,
            root: root.into(),
        }
    }

    pub(crate) fn default_remote() -> Self {
        Self::new(PeerStoreRole::RemotePeer, "$HOME/.ssh_proxy")
    }

    pub(crate) fn artifact_path(&self, artifact: PeerArtifact) -> String {
        format!(
            "{}/{}",
            self.root.trim_end_matches(['/', '\\']),
            artifact.file_name()
        )
    }

    pub(crate) fn to_redacted_value(&self) -> Value {
        json!({
            "role": self.role.as_str(),
            "root": self.root,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PeerStoreBundle {
    files: PeerConfigFiles,
    pub(crate) peer_state: Value,
    pub(crate) install_report: Value,
    pub(crate) health: Value,
    pub(crate) routes: Value,
}

impl PeerStoreBundle {
    pub(crate) fn from_config_files(files: PeerConfigFiles) -> Result<Self> {
        let peer_state = parse_store_json(
            PeerArtifact::PeerState,
            &files.peer_state_json,
            Some("ssh_proxy_peer_state.v1"),
        )?;
        let install_report = parse_store_json(
            PeerArtifact::InstallReport,
            &files.install_report_json,
            Some("ssh_proxy_remote_install.v1"),
        )?;
        let health = parse_store_json(
            PeerArtifact::Health,
            &files.health_json,
            Some("ssh_proxy_peer_health.v1"),
        )?;
        let routes = parse_routes_json(&files.routes_json)?;
        Ok(Self {
            files,
            peer_state,
            install_report,
            health,
            routes,
        })
    }

    pub(crate) fn into_artifacts(self) -> Vec<PeerArtifactBytes> {
        materialized_peer_artifacts(self.files)
    }

    pub(crate) fn to_redacted_value(&self) -> Value {
        redact_value(&json!({
            "peer_state": self.peer_state,
            "install_report": self.install_report,
            "health": self.health,
            "routes": self.routes,
        }))
    }
}

fn parse_store_json(artifact: PeerArtifact, raw: &str, schema: Option<&str>) -> Result<Value> {
    let value: Value = serde_json::from_str(raw)
        .with_context(|| format!("failed to parse {}", artifact.file_name()))?;
    if let Some(expected) = schema {
        let actual = value.get("schema").and_then(Value::as_str);
        if actual != Some(expected) {
            return Err(anyhow!(
                "{} schema mismatch: expected {expected}, got {}",
                artifact.file_name(),
                actual.unwrap_or("<missing>")
            ));
        }
    }
    Ok(value)
}

fn parse_routes_json(raw: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(raw).context("failed to parse routes.json")?;
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(anyhow!("routes.json version mismatch"));
    }
    if !value.get("routes").is_some_and(Value::is_array) {
        return Err(anyhow!("routes.json is missing routes array"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;

    fn files() -> PeerConfigFiles {
        super::super::config::materialize_peer_config(&super::super::config::PeerConfigInput {
            node_id: "spx-remote".to_string(),
            node_name: "remote".to_string(),
            token: "secret-token".to_string(),
            transport: "127.0.0.1:19080".parse::<SocketAddr>().unwrap(),
            control: "127.0.0.1:19081".parse::<SocketAddr>().unwrap(),
            local_node_id: None,
            local_node_name: None,
            local_control_endpoint: None,
            local_transport: None,
            service_manager: "systemd_user".to_string(),
            updated_at_unix: 42,
        })
    }

    #[test]
    fn store_bundle_validates_peer_state_schema() {
        let bundle = PeerStoreBundle::from_config_files(files()).unwrap();

        assert_eq!(bundle.peer_state["schema"], "ssh_proxy_peer_state.v1");
        assert_eq!(
            bundle.install_report["schema"],
            "ssh_proxy_remote_install.v1"
        );
        assert_eq!(bundle.health["schema"], "ssh_proxy_peer_health.v1");
        assert_eq!(bundle.routes["version"], 1);
        assert_eq!(bundle.into_artifacts()[0].artifact, PeerArtifact::Config);
    }

    #[test]
    fn store_bundle_rejects_incompatible_schema() {
        let mut files = files();
        files.peer_state_json = "{\"schema\":\"old\"}\n".to_string();

        let err = PeerStoreBundle::from_config_files(files).unwrap_err();

        assert!(err.to_string().contains("schema mismatch"));
    }

    #[test]
    fn store_redaction_hides_sensitive_report_fields() {
        let mut bundle = PeerStoreBundle::from_config_files(files()).unwrap();
        bundle.install_report["token"] = json!("secret");
        bundle.install_report["identity"] = json!("C:/Users/me/.ssh/id_ed25519");

        let redacted = bundle.to_redacted_value();

        assert_eq!(redacted["install_report"]["token"], "<redacted>");
        assert_eq!(
            redacted["install_report"]["identity"],
            "<redacted>/id_ed25519"
        );
    }

    #[test]
    fn store_layout_builds_symmetric_artifact_paths() {
        let layout = PeerStoreLayout::default_remote();

        assert_eq!(layout.role.as_str(), "remote_peer");
        assert_eq!(
            layout.artifact_path(PeerArtifact::InstallReport),
            "$HOME/.ssh_proxy/install_report.json"
        );
    }
}
