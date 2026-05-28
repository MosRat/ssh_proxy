use std::net::SocketAddr;

use serde_json::{Value, json};

use super::workflow::PeerLifecyclePhase;

#[derive(Debug, Clone)]
pub(crate) struct PeerConfigInput {
    pub(crate) node_id: String,
    pub(crate) node_name: String,
    pub(crate) token: String,
    pub(crate) transport: SocketAddr,
    pub(crate) control: SocketAddr,
    pub(crate) local_node_id: Option<String>,
    pub(crate) local_node_name: Option<String>,
    pub(crate) local_control_endpoint: Option<String>,
    pub(crate) local_transport: Option<SocketAddr>,
    pub(crate) service_manager: String,
    pub(crate) updated_at_unix: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct PeerConfigFiles {
    pub(crate) config_toml: String,
    pub(crate) peer_state_json: String,
    pub(crate) install_report_json: String,
    pub(crate) health_json: String,
    pub(crate) routes_json: String,
}

impl PeerConfigInput {
    pub(crate) fn control_endpoint(&self) -> String {
        format!("tcp://{}", self.control)
    }
}

pub(crate) fn materialize_peer_config(input: &PeerConfigInput) -> PeerConfigFiles {
    let peer_table = input
        .local_node_id
        .as_deref()
        .map(|node_id| {
            let mut table = String::new();
            table.push_str("\n[peers.bootstrap-local]\n");
            table.push_str(&format!("node_id = {}\n", toml_quote(node_id)));
            table.push_str(&format!(
                "node_name = {}\n",
                toml_quote(input.local_node_name.as_deref().unwrap_or("local"))
            ));
            table.push_str("trust = \"ssh-bootstrap\"\n");
            if let Some(endpoint) = input.local_control_endpoint.as_deref() {
                table.push_str(&format!("control_endpoint = {}\n", toml_quote(endpoint)));
            }
            if let Some(transport) = input.local_transport {
                table.push_str(&format!(
                    "transport = {}\n",
                    toml_quote(&transport.to_string())
                ));
            }
            table
        })
        .unwrap_or_default();
    let config_toml = format!(
        "[identity]\nnode_id = {}\nnode_name = {}\nsecret = {}\n\n[daemon]\ncontrol_endpoint = {}\ntransport_listen = {}\ntoken = {}\nroute_autostart = true\n\n[daemon.token_metadata]\ncreated_at_unix = {}\nscope = \"daemon-control-transport\"\n{}",
        toml_quote(&input.node_id),
        toml_quote(&input.node_name),
        toml_quote(&input.token),
        toml_quote(&input.control_endpoint()),
        toml_quote(&input.transport.to_string()),
        toml_quote(&input.token),
        input.updated_at_unix,
        peer_table,
    );
    let peer_state = json!({
        "schema": "ssh_proxy_peer_state.v1",
        "state": "configured",
        "service_manager": &input.service_manager,
        "transport": input.transport.to_string(),
        "control": input.control_endpoint(),
        "updated_at_unix": input.updated_at_unix,
    });
    let install_report = service_install_report_json(
        "configured",
        PeerLifecyclePhase::WriteConfig,
        &input.service_manager,
        input.updated_at_unix,
    );
    let health = peer_health_json(
        "starting",
        &input.service_manager,
        input.transport,
        input.control,
        input.updated_at_unix,
    );
    PeerConfigFiles {
        config_toml,
        peer_state_json: json_pretty_line(&peer_state),
        install_report_json: json_pretty_line(&install_report),
        health_json: json_pretty_line(&health),
        routes_json: "{\"version\":1,\"routes\":[]}\n".to_string(),
    }
}

pub(crate) fn service_install_report_json(
    state: &str,
    phase: PeerLifecyclePhase,
    service_manager: &str,
    updated_at_unix: u64,
) -> Value {
    json!({
        "schema": "ssh_proxy_remote_install.v1",
        "state": state,
        "phase": phase.as_str(),
        "service_manager": service_manager,
        "updated_at_unix": updated_at_unix,
    })
}

pub(crate) fn peer_health_json(
    state: &str,
    service_manager: &str,
    transport: SocketAddr,
    control: SocketAddr,
    updated_at_unix: u64,
) -> Value {
    json!({
        "schema": "ssh_proxy_peer_health.v1",
        "state": state,
        "service_manager": service_manager,
        "transport": transport.to_string(),
        "control": format!("tcp://{control}"),
        "updated_at_unix": updated_at_unix,
    })
}

fn json_pretty_line(value: &Value) -> String {
    let mut text = serde_json::to_string(value).expect("peer lifecycle JSON should serialize");
    text.push('\n');
    text
}

fn toml_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_config_files_are_materialized_without_shell_templates() {
        let files = materialize_peer_config(&PeerConfigInput {
            node_id: "spx-remote".to_string(),
            node_name: "user@remote".to_string(),
            token: "secret-token".to_string(),
            transport: "127.0.0.1:19080".parse().unwrap(),
            control: "127.0.0.1:19081".parse().unwrap(),
            local_node_id: Some("spx-local".to_string()),
            local_node_name: Some("local".to_string()),
            local_control_endpoint: Some("npipe://ssh_proxy/control".to_string()),
            local_transport: Some("127.0.0.1:29080".parse().unwrap()),
            service_manager: "systemd_user".to_string(),
            updated_at_unix: 42,
        });

        let parsed: toml::Value = toml::from_str(&files.config_toml).unwrap();
        assert_eq!(parsed["identity"]["node_id"].as_str(), Some("spx-remote"));
        assert_eq!(
            parsed["peers"]["bootstrap-local"]["node_id"].as_str(),
            Some("spx-local")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&files.install_report_json).unwrap()["phase"],
            "write_config"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&files.health_json).unwrap()["state"],
            "starting"
        );
        assert_eq!(files.routes_json.trim(), "{\"version\":1,\"routes\":[]}");
    }
}
