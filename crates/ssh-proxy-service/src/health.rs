use std::collections::HashSet;

use serde_json::{Map, Value, json};
use ssh_proxy_protocol::protocol_core::{
    peer::{PEER_VERSION, default_features},
    version::{compare_dotted_versions, protocol_compatibility_report},
};

#[derive(Debug, Clone)]
pub struct ConfigFileHealthInput {
    pub path: String,
    pub current_schema_version: u32,
    pub state: ConfigFileHealthState,
}

#[derive(Debug, Clone)]
pub enum ConfigFileHealthState {
    Missing,
    Valid {
        schema_version: u32,
        schema_recorded: bool,
    },
    Invalid {
        error: String,
    },
    ReadError {
        exists: bool,
        error: String,
    },
}

#[derive(Debug, Clone)]
pub struct RouteStoreHealthInput {
    pub path: String,
    pub current_version: u64,
    pub state: RouteStoreHealthState,
}

#[derive(Debug, Clone)]
pub enum RouteStoreHealthState {
    Missing,
    Loaded(Value),
    Error { exists: bool, error: String },
}

#[derive(Debug, Clone)]
pub struct BinaryHealthInput {
    pub source_exists: bool,
    pub installed_exists: bool,
    pub copy_exe: bool,
    pub same_path: bool,
}

#[derive(Debug, Clone)]
pub enum EndpointHealthInput {
    Control {
        endpoint: String,
        kind: Option<String>,
        error: Option<String>,
    },
    MissingPeerControl,
    Tcp {
        addr: Option<String>,
        probe_addr: Option<String>,
        reachable: Option<bool>,
        error: Option<String>,
    },
    Quic {
        addr: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ServiceHealthInput {
    pub config: Value,
    pub route_store: Value,
    pub binary: Value,
    pub control: Value,
    pub plain_tcp: Value,
    pub tls_tcp: Value,
    pub quic: Value,
    pub peers: Value,
}

#[derive(Debug, Clone)]
pub struct PeerRegistryHealthInput {
    pub config_available: bool,
    pub peers: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct PeerHealthInput {
    pub alias: String,
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub version: Option<String>,
    pub control_api_version: Option<u16>,
    pub peer_protocol_version: Option<u16>,
    pub features: Vec<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub target: Option<String>,
    pub trust: Option<String>,
    pub protocols: Vec<String>,
    pub token_present: bool,
    pub token_metadata: Value,
    pub compatibility: Value,
    pub last_seen_unix: Option<u64>,
    pub control: Value,
    pub plain_tcp: Value,
    pub tls_tcp: Value,
    pub quic: Value,
}

#[derive(Debug, Clone)]
pub struct PeerCompatibilityInput {
    pub local_version: String,
    pub local_control_api_version: u16,
    pub local_peer_protocol_version: u16,
    pub local_features: Vec<String>,
    pub remote_version: Option<String>,
    pub remote_control_api_version: Option<u16>,
    pub remote_peer_protocol_version: Option<u16>,
    pub remote_features: Vec<String>,
    pub remote_os: Option<String>,
    pub remote_arch: Option<String>,
}

impl PeerCompatibilityInput {
    pub fn current_local(remote_version: Option<String>) -> Self {
        Self {
            local_version: env!("CARGO_PKG_VERSION").to_string(),
            local_control_api_version:
                ssh_proxy_protocol::protocol_core::version::CONTROL_API_VERSION,
            local_peer_protocol_version: PEER_VERSION,
            local_features: default_features(),
            remote_version,
            remote_control_api_version: None,
            remote_peer_protocol_version: None,
            remote_features: Vec::new(),
            remote_os: None,
            remote_arch: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerVersionCheckInput {
    pub ok: Option<bool>,
    pub kind: String,
    pub alias: String,
    pub target: Option<String>,
    pub recorded: Option<bool>,
    pub fresh: Option<bool>,
    pub compatibility: PeerCompatibilityInput,
}

#[derive(Debug, Clone, Copy)]
enum BinaryVersionMessageStyle {
    ServiceHealth,
    VersionCheck,
}

pub fn service_health_report(input: ServiceHealthInput) -> Value {
    json!({
        "config": input.config,
        "route_store": input.route_store,
        "binary": input.binary,
        "listeners": {
            "control": input.control,
            "plain_tcp": input.plain_tcp,
            "tls_tcp": input.tls_tcp,
            "quic": input.quic,
        },
        "peers": input.peers,
    })
}

pub fn config_file_health_report(input: ConfigFileHealthInput) -> Value {
    match input.state {
        ConfigFileHealthState::Missing => json!({
            "ok": false,
            "exists": false,
            "path": input.path,
            "message": "config file does not exist; run `ssh_proxy config init` or `ssh_proxy service install`",
            "next_action": "config-init",
            "repair_action": "ssh_proxy config init",
        }),
        ConfigFileHealthState::Valid {
            schema_version,
            schema_recorded,
        } => json!({
            "ok": true,
            "exists": true,
            "path": input.path,
            "schema_version": schema_version,
            "current_schema_version": input.current_schema_version,
            "schema_recorded": schema_recorded,
            "legacy_without_schema": !schema_recorded,
            "next_action": "none",
            "repair_action": Value::Null,
        }),
        ConfigFileHealthState::Invalid { error } => json!({
            "ok": false,
            "exists": true,
            "path": input.path,
            "current_schema_version": input.current_schema_version,
            "error": error,
            "next_action": "repair-config",
            "repair_action": "ssh_proxy config inspect",
        }),
        ConfigFileHealthState::ReadError { exists, error } => json!({
            "ok": false,
            "exists": exists,
            "path": input.path,
            "error": error,
            "next_action": "repair-config",
            "repair_action": "check file permissions and rerun ssh_proxy service status",
        }),
    }
}

pub fn route_store_health_report(input: RouteStoreHealthInput) -> Value {
    match input.state {
        RouteStoreHealthState::Missing => json!({
            "ok": true,
            "exists": false,
            "path": input.path,
            "routes": 0,
            "message": "route store does not exist yet",
            "next_action": "none",
            "repair_action": Value::Null,
        }),
        RouteStoreHealthState::Loaded(value) => {
            let version = value.get("version").and_then(Value::as_u64);
            let routes_array = value.get("routes").and_then(Value::as_array);
            let routes = routes_array.map(Vec::len);
            let duplicate_ids = duplicate_route_ids(routes_array);
            let ok = version == Some(input.current_version)
                && routes.is_some()
                && duplicate_ids.is_empty();
            json!({
                "ok": ok,
                "exists": true,
                "path": input.path,
                "version": version,
                "current_version": input.current_version,
                "routes": routes,
                "duplicate_ids": duplicate_ids,
                "next_action": if ok { "none" } else { "repair-route-store" },
                "repair_action": if ok {
                    Value::Null
                } else {
                    json!("inspect routes.json and remove duplicate or incompatible records")
                },
            })
        }
        RouteStoreHealthState::Error { exists, error } => json!({
            "ok": false,
            "exists": exists,
            "path": input.path,
            "error": error,
            "next_action": "repair-route-store",
            "repair_action": "inspect routes.json or remove it after stopping the daemon",
        }),
    }
}

pub fn binary_health_report(input: BinaryHealthInput) -> Value {
    let ok = input.source_exists && (input.installed_exists || !input.copy_exe || input.same_path);
    json!({
        "ok": ok,
        "source_exists": input.source_exists,
        "installed_exists": input.installed_exists,
        "copy_exe": input.copy_exe,
        "same_path": input.same_path,
        "next_action": if ok { "none" } else { "service-install" },
        "repair_action": if ok { Value::Null } else { json!("ssh_proxy service install") },
    })
}

pub fn endpoint_health_report(input: EndpointHealthInput) -> Value {
    match input {
        EndpointHealthInput::Control {
            endpoint,
            kind: Some(kind),
            error: None,
        } => json!({
            "ok": true,
            "endpoint": endpoint,
            "kind": kind,
            "next_action": "none",
            "repair_action": Value::Null,
        }),
        EndpointHealthInput::Control {
            endpoint, error, ..
        } => json!({
            "ok": false,
            "endpoint": endpoint,
            "error": error.unwrap_or_else(|| "control endpoint could not be parsed".to_string()),
            "next_action": "repair-control-endpoint",
            "repair_action": "check daemon control endpoint configuration",
        }),
        EndpointHealthInput::MissingPeerControl => json!({
            "ok": false,
            "configured": false,
            "reachable": Value::Null,
            "message": "peer control endpoint is not recorded",
            "next_action": "peer-refresh",
            "repair_action": "ssh_proxy peer refresh",
        }),
        EndpointHealthInput::Tcp { addr: None, .. } => json!({
            "ok": true,
            "configured": false,
            "reachable": Value::Null,
            "next_action": "none",
            "repair_action": Value::Null,
        }),
        EndpointHealthInput::Tcp {
            addr: Some(addr),
            probe_addr,
            reachable,
            error,
        } => {
            let reachable = reachable.unwrap_or(false);
            json!({
                "ok": reachable,
                "configured": true,
                "addr": addr,
                "probe_addr": probe_addr,
                "reachable": reachable,
                "error": error,
                "next_action": if reachable { "none" } else { "repair-listener" },
                "repair_action": if reachable {
                    Value::Null
                } else {
                    json!("restart the daemon or refresh the peer transport")
                },
            })
        }
        EndpointHealthInput::Quic { addr: Some(addr) } => json!({
            "ok": true,
            "configured": true,
            "addr": addr,
            "reachable": Value::Null,
            "message": "QUIC UDP listener reachability is not probed by service status yet",
            "next_action": "none",
            "repair_action": Value::Null,
        }),
        EndpointHealthInput::Quic { addr: None } => json!({
            "ok": true,
            "configured": false,
            "reachable": Value::Null,
            "next_action": "none",
            "repair_action": Value::Null,
        }),
    }
}

pub fn peer_registry_health_report(input: PeerRegistryHealthInput) -> Value {
    if !input.config_available {
        return json!({
            "ok": false,
            "count": 0,
            "error": "config unavailable",
            "peers": [],
            "next_action": "repair-config",
            "repair_action": "ssh_proxy config inspect",
        });
    }

    let ok = input
        .peers
        .iter()
        .all(|summary| summary.get("ok").and_then(Value::as_bool).unwrap_or(false));
    json!({
        "ok": ok,
        "count": input.peers.len(),
        "peers": input.peers,
        "next_action": if ok { "none" } else { "peer-refresh" },
        "repair_action": if ok { Value::Null } else { json!("ssh_proxy peer refresh") },
    })
}

pub fn peer_health_report(input: PeerHealthInput) -> Value {
    let ok = [
        &input.control,
        &input.plain_tcp,
        &input.tls_tcp,
        &input.quic,
        &input.compatibility,
    ]
    .iter()
    .all(|value| value.get("ok").and_then(Value::as_bool).unwrap_or(false));
    json!({
        "ok": ok,
        "alias": input.alias,
        "node_id": input.node_id,
        "node_name": input.node_name,
        "version": input.version,
        "control_api_version": input.control_api_version,
        "peer_protocol_version": input.peer_protocol_version,
        "features": input.features,
        "os": input.os,
        "arch": input.arch,
        "target": input.target,
        "trust": input.trust,
        "protocols": input.protocols,
        "auth": {
            "token": input.token_present,
            "token_metadata": input.token_metadata,
        },
        "compatibility": input.compatibility,
        "last_seen_unix": input.last_seen_unix,
        "endpoints": {
            "control": input.control,
            "plain_tcp": input.plain_tcp,
            "tls_tcp": input.tls_tcp,
            "quic": input.quic,
        },
        "next_action": if ok { "none" } else { "peer-refresh" },
        "repair_action": if ok { Value::Null } else { json!("ssh_proxy peer refresh") },
    })
}

pub fn peer_compatibility_report(input: PeerCompatibilityInput) -> Value {
    let components = compatibility_components(&input, BinaryVersionMessageStyle::ServiceHealth);
    json!({
        "ok": components.compatible,
        "status": if components.compatible { "compatible" } else { "incompatible" },
        "local": {
            "version": input.local_version,
            "control_api_version": input.local_control_api_version,
            "peer_protocol_version": input.local_peer_protocol_version,
            "features": input.local_features,
        },
        "remote": {
            "version": input.remote_version,
            "control_api_version": input.remote_control_api_version,
            "peer_protocol_version": input.remote_peer_protocol_version,
            "features": input.remote_features,
            "common_features": components.common_features,
            "missing_features": components.missing_features,
            "os": input.remote_os,
            "arch": input.remote_arch,
        },
        "checks": components.checks,
        "next_action": service_compatibility_next_action(&input, components.compatible),
        "repair_action": compatibility_repair_action(service_compatibility_next_action(&input, components.compatible)),
    })
}

pub fn peer_version_check_report(input: PeerVersionCheckInput) -> Value {
    let components = compatibility_components(
        &input.compatibility,
        BinaryVersionMessageStyle::VersionCheck,
    );
    let next_action = version_next_action(&input.compatibility, &components.missing_features);
    let status = version_status(
        components.compatible,
        &input.compatibility.local_version,
        input.compatibility.remote_version.as_deref(),
        next_action,
    );
    let mut object = Map::new();
    if let Some(ok) = input.ok {
        object.insert("ok".to_string(), json!(ok));
    }
    object.insert("kind".to_string(), json!(input.kind));
    object.insert("alias".to_string(), json!(input.alias));
    if let Some(target) = input.target {
        object.insert("target".to_string(), json!(target));
    }
    if let Some(recorded) = input.recorded {
        object.insert("recorded".to_string(), json!(recorded));
    }
    if let Some(fresh) = input.fresh {
        object.insert("fresh".to_string(), json!(fresh));
    }
    object.insert("compatible".to_string(), json!(components.compatible));
    object.insert("status".to_string(), json!(status));
    object.insert(
        "local".to_string(),
        json!({
            "version": input.compatibility.local_version,
            "control_api_version": input.compatibility.local_control_api_version,
            "peer_protocol_version": input.compatibility.local_peer_protocol_version,
            "features": input.compatibility.local_features,
        }),
    );
    object.insert(
        "remote".to_string(),
        json!({
            "version": input.compatibility.remote_version,
            "control_api_version": input.compatibility.remote_control_api_version,
            "peer_protocol_version": input.compatibility.remote_peer_protocol_version,
            "features": input.compatibility.remote_features,
            "common_features": components.common_features,
            "missing_features": components.missing_features,
            "os": input.compatibility.remote_os,
            "arch": input.compatibility.remote_arch,
        }),
    );
    object.insert("checks".to_string(), json!(components.checks));
    object.insert("next_action".to_string(), json!(next_action));
    Value::Object(object)
}

pub fn unrecorded_peer_version_check_report(alias: impl Into<String>) -> Value {
    json!({
        "kind": "saved_peer_version_check",
        "alias": alias.into(),
        "recorded": false,
        "compatible": false,
        "status": "unrecorded",
        "checks": [],
        "next_action": "peer-bootstrap",
        "message": "peer is not recorded locally; route start will try descriptor adoption then SSH bootstrap"
    })
}

fn duplicate_route_ids(routes: Option<&Vec<Value>>) -> Vec<String> {
    let Some(routes) = routes else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for route in routes {
        let Some(id) = route.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(id.to_string()) && !duplicates.iter().any(|duplicate| duplicate == id) {
            duplicates.push(id.to_string());
        }
    }
    duplicates
}

struct CompatibilityComponents {
    checks: Vec<Value>,
    compatible: bool,
    missing_features: Vec<String>,
    common_features: Vec<String>,
}

fn compatibility_components(
    input: &PeerCompatibilityInput,
    style: BinaryVersionMessageStyle,
) -> CompatibilityComponents {
    let protocol_report = protocol_compatibility_report(
        input.local_control_api_version,
        input.remote_control_api_version,
        input.local_peer_protocol_version,
        input.remote_peer_protocol_version,
        &input.local_features,
        &input.remote_features,
    );
    let mut checks = protocol_report.checks;
    checks.push(binary_version_check(
        &input.local_version,
        input.remote_version.as_deref(),
        style,
    ));
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    CompatibilityComponents {
        checks,
        compatible,
        missing_features: protocol_report.missing_features,
        common_features: protocol_report.common_features,
    }
}

fn binary_version_check(
    local: &str,
    remote: Option<&str>,
    style: BinaryVersionMessageStyle,
) -> Value {
    match remote.and_then(|remote| compare_dotted_versions(local, remote)) {
        Some(std::cmp::Ordering::Equal) => match style {
            BinaryVersionMessageStyle::ServiceHealth => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "info",
            }),
            BinaryVersionMessageStyle::VersionCheck => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "info",
                "message": "local and remote binaries report the same package version"
            }),
        },
        Some(std::cmp::Ordering::Greater) => match style {
            BinaryVersionMessageStyle::ServiceHealth => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "warning",
                "message": "saved peer binary is older; refresh or bootstrap the peer when changing protocols",
            }),
            BinaryVersionMessageStyle::VersionCheck => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "warning",
                "message": "remote binary is older; bootstrap with --force to align versions"
            }),
        },
        Some(std::cmp::Ordering::Less) => match style {
            BinaryVersionMessageStyle::ServiceHealth => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "warning",
                "message": "saved peer binary is newer; consider upgrading the local binary",
            }),
            BinaryVersionMessageStyle::VersionCheck => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "warning",
                "message": "remote binary is newer; consider upgrading the local binary"
            }),
        },
        None => match style {
            BinaryVersionMessageStyle::ServiceHealth => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "warning",
                "message": "binary version is not recorded or cannot be compared",
            }),
            BinaryVersionMessageStyle::VersionCheck => json!({
                "name": "binary_version",
                "ok": true,
                "local": local,
                "remote": remote,
                "severity": "warning",
                "message": "binary version could not be compared"
            }),
        },
    }
}

fn service_compatibility_next_action(
    input: &PeerCompatibilityInput,
    compatible: bool,
) -> &'static str {
    if !compatible {
        if input
            .remote_control_api_version
            .is_some_and(|remote| remote > input.local_control_api_version)
            || input
                .remote_peer_protocol_version
                .is_some_and(|remote| remote > input.local_peer_protocol_version)
        {
            "upgrade-local"
        } else {
            "peer-refresh"
        }
    } else if input.remote_version.as_deref().is_some_and(|remote| {
        compare_dotted_versions(&input.local_version, remote) == Some(std::cmp::Ordering::Greater)
    }) {
        "peer-bootstrap --force"
    } else {
        "none"
    }
}

fn compatibility_repair_action(next_action: &str) -> Value {
    match next_action {
        "upgrade-local" => json!("upgrade the local ssh_proxy binary"),
        "peer-refresh" => json!("ssh_proxy peer refresh"),
        "peer-bootstrap --force" => json!("ssh_proxy peer bootstrap --force"),
        _ => Value::Null,
    }
}

fn version_next_action(
    input: &PeerCompatibilityInput,
    missing_features: &[String],
) -> &'static str {
    if input
        .remote_control_api_version
        .is_some_and(|remote| remote > input.local_control_api_version)
        || input
            .remote_peer_protocol_version
            .is_some_and(|remote| remote > input.local_peer_protocol_version)
    {
        return "upgrade-local";
    }
    if input.remote_control_api_version.is_none()
        || input.remote_peer_protocol_version.is_none()
        || input
            .remote_peer_protocol_version
            .is_some_and(|remote| remote < input.local_peer_protocol_version)
        || !missing_features.is_empty()
    {
        return "peer-bootstrap --force";
    }
    match input
        .remote_version
        .as_deref()
        .and_then(|remote| compare_dotted_versions(&input.local_version, remote))
    {
        Some(std::cmp::Ordering::Greater) => "peer-bootstrap --force",
        Some(std::cmp::Ordering::Less) => "upgrade-local",
        _ => "none",
    }
}

fn version_status(
    compatible: bool,
    local_version: &str,
    remote_version: Option<&str>,
    next_action: &str,
) -> &'static str {
    if !compatible {
        return "incompatible";
    }
    match remote_version.and_then(|remote| compare_dotted_versions(local_version, remote)) {
        Some(std::cmp::Ordering::Equal) => "compatible",
        Some(std::cmp::Ordering::Greater) if next_action == "peer-bootstrap --force" => {
            "compatible-upgrade-remote"
        }
        Some(std::cmp::Ordering::Less) if next_action == "upgrade-local" => {
            "compatible-upgrade-local"
        }
        _ => "compatible-version-unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compatibility_input(remote_version: Option<&str>) -> PeerCompatibilityInput {
        PeerCompatibilityInput {
            local_version: "0.2.0".to_string(),
            local_control_api_version: 1,
            local_peer_protocol_version: 1,
            local_features: vec!["frames-v1".to_string(), "tcp-connect".to_string()],
            remote_version: remote_version.map(ToOwned::to_owned),
            remote_control_api_version: Some(1),
            remote_peer_protocol_version: Some(1),
            remote_features: vec!["frames-v1".to_string(), "tcp-connect".to_string()],
            remote_os: Some("linux".to_string()),
            remote_arch: Some("x86_64".to_string()),
        }
    }

    #[test]
    fn route_store_report_flags_duplicate_ids() {
        let report = route_store_health_report(RouteStoreHealthInput {
            path: "routes.json".to_string(),
            current_version: 1,
            state: RouteStoreHealthState::Loaded(json!({
                "version": 1,
                "routes": [{"id": "a"}, {"id": "a"}],
            })),
        });

        assert_eq!(report["ok"], false);
        assert_eq!(report["duplicate_ids"][0], "a");
        assert_eq!(report["next_action"], "repair-route-store");
    }

    #[test]
    fn endpoint_report_preserves_tcp_probe_shape() {
        let report = endpoint_health_report(EndpointHealthInput::Tcp {
            addr: Some("0.0.0.0:19080".to_string()),
            probe_addr: Some("127.0.0.1:19080".to_string()),
            reachable: Some(false),
            error: Some("refused".to_string()),
        });

        assert_eq!(report["configured"], true);
        assert_eq!(report["reachable"], false);
        assert_eq!(report["next_action"], "repair-listener");
    }

    #[test]
    fn service_peer_compatibility_reports_upgrade_action() {
        let mut input = compatibility_input(Some("0.2.0"));
        input.remote_control_api_version = Some(2);
        let report = peer_compatibility_report(input);

        assert_eq!(report["ok"], false);
        assert_eq!(report["next_action"], "upgrade-local");
        assert_eq!(
            report["remote"]["missing_features"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn version_check_preserves_saved_peer_shape() {
        let report = peer_version_check_report(PeerVersionCheckInput {
            ok: None,
            kind: "saved_peer_version_check".to_string(),
            alias: "edge".to_string(),
            target: None,
            recorded: Some(true),
            fresh: Some(false),
            compatibility: compatibility_input(Some("0.1.0")),
        });

        assert_eq!(report["kind"], "saved_peer_version_check");
        assert_eq!(report["alias"], "edge");
        assert_eq!(report["recorded"], true);
        assert_eq!(report["next_action"], "peer-bootstrap --force");
    }
}
