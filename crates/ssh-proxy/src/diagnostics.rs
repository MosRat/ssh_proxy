use std::{env, fs, path::PathBuf, process::Command};

use serde_json::{Map, Value, json};

use crate::{config, control_socket, paths, peer_lifecycle, repair};

pub(crate) fn daemon_dependency_report(
    config: &config::AppConfig,
    status: Option<&Value>,
) -> Value {
    let control = status
        .and_then(|status| status.get("control").and_then(Value::as_str))
        .map(str::to_string)
        .or_else(|| config.daemon.control_endpoint.clone())
        .unwrap_or_else(control_socket::default_endpoint_string);
    let daemon_ok = status
        .and_then(|status| status.get("ok").and_then(Value::as_bool))
        .unwrap_or(false);
    let mut dependencies = Vec::new();
    dependencies.push(dependency(
        "daemon_control",
        "required",
        if daemon_ok { "ok" } else { "blocked" },
        Some(if daemon_ok {
            "daemon control endpoint responded"
        } else {
            "daemon control endpoint is unavailable"
        }),
        (!daemon_ok).then_some("daemon_unavailable"),
    ));
    dependencies.push(dependency(
        "private_control_endpoint",
        "required",
        if is_private_control_endpoint(&control) {
            "ok"
        } else {
            "blocked"
        },
        Some(&control),
        (!is_private_control_endpoint(&control)).then_some("daemon_pipe_access_denied"),
    ));
    dependencies.push(platform_service_dependency());
    dependencies.push(install_dir_dependency());
    dependencies.push(ssh_agent_dependency());
    dependencies.push(dependency(
        "rust_ssh_config",
        "required",
        "ok",
        Some("supports HostName, User, Port, IdentityFile, UserKnownHostsFile, StrictHostKeyChecking=accept-new/no, and ProxyJump; ProxyCommand requires explicit emergency compatibility"),
        None,
    ));
    dependencies.push(dependency(
        "remote_shell",
        "required",
        "checked_during_job",
        Some("remote settings and peer lifecycle still require a POSIX shell on Linux targets"),
        None,
    ));
    dependencies.push(dependency(
        "remote_git",
        "optional",
        "checked_during_job",
        Some("Git proxy configuration is applied only when git is available and enabled"),
        None,
    ));
    dependencies.push(dependency(
        "remote_node",
        "diagnostic_only",
        "not_required",
        Some("normal remote setup no longer requires remote node for settings JSON changes"),
        None,
    ));
    dependencies.push(dependency(
        "remote_systemd_nohup",
        "optional",
        "checked_during_peer_management",
        Some("remote peer lifecycle reports systemd/nohup capability when peer bootstrap/update runs"),
        None,
    ));
    json!(dependencies)
}

pub(crate) fn doctor_report(config: &config::AppConfig, status: Option<Value>) -> Value {
    let dependencies = daemon_dependency_report(config, status.as_ref());
    let redacted_status = status.as_ref().map(redact_value).unwrap_or(Value::Null);
    json!({
        "schema": "ssh_proxy_doctor_report.v1",
        "generated_at_unix": now_unix(),
        "version": env!("CARGO_PKG_VERSION"),
        "os": env::consts::OS,
        "arch": env::consts::ARCH,
        "dependencies": dependencies,
        "daemon_status": redacted_status,
        "recent_install_logs": recent_install_logs(),
    })
}

pub(crate) fn peer_report(
    config: &config::AppConfig,
    status: Option<&Value>,
    target: &str,
) -> Value {
    let config_peer = config.peers.get(target);
    let status_peer = status
        .and_then(|status| status.get("peer_store"))
        .and_then(Value::as_array)
        .and_then(|peers| {
            peers
                .iter()
                .find(|peer| peer.get("target").and_then(Value::as_str) == Some(target))
        })
        .cloned()
        .unwrap_or(Value::Null);
    let route_decisions = status
        .and_then(|status| status.get("routes"))
        .and_then(Value::as_array)
        .map(|routes| {
            routes
                .iter()
                .filter(|route| route.to_string().contains(target))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "target": target,
        "config_peer": config_peer.map(|peer| redact_value(&json!({
            "node_id": peer.node_id,
            "node_name": peer.node_name,
            "service_instance_id": peer.service_instance_id,
            "version": peer.version,
            "control_api_version": peer.control_api_version,
            "peer_protocol_version": peer.peer_protocol_version,
            "features": peer.features,
            "os": peer.os,
            "arch": peer.arch,
            "remote_path": peer.remote_path,
            "control_endpoint": peer.control_endpoint,
            "transport": peer.transport.map(|addr| addr.to_string()),
            "tls_transport": peer.tls_transport.map(|addr| addr.to_string()),
            "quic_transport": peer.quic_transport.map(|addr| addr.to_string()),
            "transport_protocols": peer.known_transport_protocols(),
            "auth": {
                "token": peer.token.is_some(),
                "token_metadata": peer.token_metadata,
                "tls_server_cert_fingerprint": peer.tls_server_cert_fingerprint,
                "tls_client_ca_fingerprint": peer.tls_client_ca_fingerprint,
            },
            "last_seen_unix": peer.last_seen_unix,
        }))),
        "daemon_peer": redact_value(&status_peer),
        "route_decisions": redact_value(&json!(route_decisions)),
        "dependency_report": [
            {
                "name": "remote_peer_server",
                "classification": "required",
                "state": if status_peer.is_null() { "not_recorded" } else { "recorded" },
                "message": "daemon-owned remote peer state used by up/vscode up"
            },
            {
                "name": "remote_service_manager",
                "classification": "optional",
                "state": status_peer.get("service_manager").and_then(Value::as_str).unwrap_or("unknown"),
                "message": "Linux prefers user systemd and falls back to managed nohup"
            }
        ],
    })
}

pub(crate) fn redact_value(value: &Value) -> Value {
    peer_lifecycle::report::redact_value(value)
}

fn dependency(
    name: &str,
    classification: &str,
    state: &str,
    message: Option<&str>,
    blocker: Option<&str>,
) -> Value {
    let mut object = Map::new();
    object.insert("name".to_string(), json!(name));
    object.insert("classification".to_string(), json!(classification));
    object.insert("state".to_string(), json!(state));
    if let Some(message) = message {
        object.insert("message".to_string(), json!(message));
    }
    if let Some(blocker) = blocker {
        object.insert("blocker".to_string(), json!(blocker));
        repair::attach_repair_action(&mut object, blocker);
    }
    Value::Object(object)
}

fn is_private_control_endpoint(endpoint: &str) -> bool {
    endpoint.starts_with(r"\\.\pipe\")
        || endpoint.starts_with("npipe://")
        || endpoint.starts_with("unix://")
        || endpoint.starts_with('/')
}

fn platform_service_dependency() -> Value {
    if cfg!(windows) {
        dependency(
            "windows_scm",
            "required",
            if program_available("sc.exe") {
                "ok"
            } else {
                "blocked"
            },
            Some("Windows Service Control Manager is required for the production system daemon"),
            (!program_available("sc.exe")).then_some("requires_elevation"),
        )
    } else {
        dependency(
            "system_service_manager",
            "optional",
            if program_available("systemctl") {
                "ok"
            } else {
                "not_available"
            },
            Some("non-Windows service management is kept for compatibility and operations"),
            None,
        )
    }
}

fn install_dir_dependency() -> Value {
    let dir = production_install_root();
    let state = dir
        .as_ref()
        .map(|dir| if dir.exists() { "ok" } else { "not_created" })
        .unwrap_or("unknown");
    dependency(
        "daemon_install_dir",
        "required",
        state,
        dir.as_ref().and_then(|path| path.to_str()),
        None,
    )
}

fn production_install_root() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("ProgramData")
            .map(PathBuf::from)
            .map(|root| root.join("ssh_proxy"))
    } else {
        paths::app_home().ok()
    }
}

fn ssh_agent_dependency() -> Value {
    let available = if cfg!(windows) {
        env::var_os("SSH_AUTH_SOCK").is_some()
            || PathBuf::from(r"\\.\pipe\openssh-ssh-agent").exists()
    } else {
        env::var_os("SSH_AUTH_SOCK").is_some()
    };
    dependency(
        "ssh_agent",
        "optional",
        if available {
            "available"
        } else {
            "not_detected"
        },
        Some(
            "Rust SSH can use ssh-agent/Pageant/OpenSSH agent before falling back to unencrypted identity files",
        ),
        None,
    )
}

fn program_available(program: &str) -> bool {
    Command::new(program)
        .arg(if cfg!(windows) { "/?" } else { "--version" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn recent_install_logs() -> Value {
    let Ok(entries) = fs::read_dir(env::temp_dir()) else {
        return json!([]);
    };
    let mut logs = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            (name.starts_with("ssh_proxy-daemon-install-") && name.ends_with(".jsonl"))
                .then_some(path)
        })
        .collect::<Vec<_>>();
    logs.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
    logs.reverse();
    Value::Array(
        logs.into_iter()
            .take(3)
            .map(|path| {
                let tail = fs::read_to_string(&path)
                    .ok()
                    .map(|text| {
                        text.lines()
                            .rev()
                            .take(8)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                json!({
                    "path": path.display().to_string(),
                    "tail": tail,
                })
            })
            .collect(),
    )
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens_and_identity_paths() {
        let value = json!({
            "token": "abc",
            "ssh": {
                "identity": ["C:/Users/me/.ssh/id_rsa"],
                "known_hosts": "C:/Users/me/.ssh/known_hosts"
            }
        });
        let redacted = redact_value(&value);
        assert_eq!(redacted["token"], "<redacted>");
        assert_eq!(redacted["ssh"]["identity"][0], "<redacted>/id_rsa");
        assert_eq!(redacted["ssh"]["known_hosts"], "<redacted>/known_hosts");
    }

    #[test]
    fn private_endpoint_detects_named_pipe() {
        assert!(is_private_control_endpoint(
            r"\\.\pipe\ssh_proxy\whl\control"
        ));
        assert!(is_private_control_endpoint("npipe://ssh_proxy/whl/control"));
        assert!(!is_private_control_endpoint("tcp://127.0.0.1:19080"));
    }
}
