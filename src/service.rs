use std::{
    collections::HashSet,
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::{self, Duration};

use crate::{cli, config};

mod broker;
mod inventory;
mod peer_health;
mod plan;
mod platform;

use inventory::{ServiceNextAction, inventory_json};
use plan::ServicePlan;

pub async fn run(args: cli::ServiceArgs, config: config::AppConfig) -> Result<()> {
    let json = args.json;
    let plan = ServicePlan::new(args, config)?;
    match plan.command {
        cli::ServiceCommand::Print => print_service(&plan),
        cli::ServiceCommand::Ensure => ensure_service(&plan, json).await,
        cli::ServiceCommand::Install => install_service(&plan),
        cli::ServiceCommand::Uninstall => platform::platform_uninstall(&plan),
        cli::ServiceCommand::Start => platform::platform_start(&plan),
        cli::ServiceCommand::Stop => platform::platform_stop(&plan),
        cli::ServiceCommand::Status => status_service(&plan, json).await,
    }
}

async fn ensure_service(plan: &ServicePlan, json_output: bool) -> Result<()> {
    let before = service_status_summary(plan).await?;
    if before["ok"].as_bool().unwrap_or(false) {
        if json_output {
            println!("{}", serde_json::to_string(&before)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&before)?);
        }
        return Ok(());
    }

    let outcome = match install_or_repair_service(plan) {
        Ok(()) => service_status_summary(plan).await?,
        Err(err) => {
            let mut value = service_status_summary(plan).await?;
            if let Some(object) = value.as_object_mut() {
                object.insert("ok".to_string(), Value::Bool(false));
                object.insert("ensure_error".to_string(), Value::String(err.to_string()));
                object.insert(
                    "next_action".to_string(),
                    Value::String(if is_permission_denied_error(&err.to_string()) {
                        "session_daemon".to_string()
                    } else if requires_elevation(plan, &err.to_string()) {
                        "install_system_elevated".to_string()
                    } else {
                        "session_daemon".to_string()
                    }),
                );
                object.insert(
                    "requires_elevation".to_string(),
                    Value::Bool(requires_elevation(plan, &err.to_string())),
                );
            }
            value
        }
    };

    if json_output {
        println!("{}", serde_json::to_string(&outcome)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    }
    Ok(())
}

fn print_service(plan: &ServicePlan) -> Result<()> {
    println!("ssh_proxy {}", env!("CARGO_PKG_VERSION"));
    println!("config: {}", plan.config_path.display());
    println!("scope: {:?}", plan.scope);
    if plan.copy_exe {
        println!("installed binary: {}", plan.exe.display());
    }
    println!("daemon command:");
    println!("  {}", plan.daemon_command());
    if let Some(transport) = plan.transport {
        println!("transport: tcp://{transport}");
    } else {
        println!("transport: disabled");
    }
    if let Some(transport) = plan.tls_transport {
        println!("tls transport: tls://{transport}");
    }
    if let Some(transport) = plan.quic_transport {
        println!("quic transport: quic://{transport}");
    }
    println!();
    platform::platform_print(plan)
}

fn install_service(plan: &ServicePlan) -> Result<()> {
    install_or_repair_service(plan)
}

fn install_or_repair_service(plan: &ServicePlan) -> Result<()> {
    let action = plan.resolution.next_action;
    if matches!(
        action,
        ServiceNextAction::Reuse | ServiceNextAction::StartOrRepair | ServiceNextAction::Install
    ) && platform::platform_install_requires_elevation(plan)
    {
        return platform::platform_install(plan);
    }
    let original_config = if plan.config_to_save.is_some() {
        match fs::read(&plan.config_path) {
            Ok(bytes) => Some(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(None),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed to snapshot existing config {}",
                        plan.config_path.display()
                    )
                });
            }
        }
    } else {
        None
    };

    match action {
        ServiceNextAction::Reuse if !matches!(plan.command, cli::ServiceCommand::Install) => {
            println!(
                "selected existing {} service at {}; no install required",
                service_scope_name(plan.scope),
                platform_service_name(plan.scope)
            );
            return Ok(());
        }
        ServiceNextAction::Reuse
        | ServiceNextAction::StartOrRepair
        | ServiceNextAction::Install => {
            let install_result = (|| -> Result<()> {
                if let Some(config) = &plan.config_to_save {
                    config.save_default()?;
                    println!("saved daemon defaults to {}", plan.config_path.display());
                }
                platform::platform_prepare_install(plan)?;
                plan.install_binary()?;
                platform::platform_install(plan)
            })();
            if let Err(err) = install_result {
                if let Some(snapshot) = original_config {
                    restore_config_snapshot(&plan.config_path, snapshot)?;
                    eprintln!(
                        "rolled back daemon defaults in {} after service install failure",
                        plan.config_path.display()
                    );
                }
                return Err(err);
            }
        }
        ServiceNextAction::Unavailable => {
            if let Some(snapshot) = original_config {
                restore_config_snapshot(&plan.config_path, snapshot)?;
            }
            return Err(anyhow::anyhow!(
                "no persistent service scope could be selected; no install target available"
            ));
        }
    }
    Ok(())
}

fn requires_elevation(plan: &ServicePlan, error: &str) -> bool {
    matches!(plan.scope, plan::ServiceScope::System)
        && !plan::is_admin()
        && (error.contains("administrator")
            || error.contains("root")
            || is_permission_denied_error(error))
}

fn is_permission_denied_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("privilege")
        || lower.contains("elevation")
}

fn restore_config_snapshot(path: &Path, snapshot: Option<Vec<u8>>) -> Result<()> {
    match snapshot {
        Some(bytes) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(path, bytes)
                .with_context(|| format!("failed to restore {}", path.display()))?;
        }
        None => match fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("failed to remove {}", path.display()));
            }
        },
    }
    Ok(())
}

async fn status_service(plan: &ServicePlan, json: bool) -> Result<()> {
    let summary = service_status_summary(plan).await?;
    if json {
        println!("{}", serde_json::to_string(&summary)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    }
    Ok(())
}

async fn service_status_summary(plan: &ServicePlan) -> Result<Value> {
    let daemon = query_daemon_status(plan).await;
    let health = service_health(plan).await;
    let platform = platform::platform_status_summary(plan);
    let daemon_reachable = daemon["reachable"].as_bool().unwrap_or(false);
    let platform_ok = platform["ok"].as_bool().unwrap_or(false);
    let overall_ok = daemon_reachable || platform_ok;
    let manager = service_manager_summary(plan, daemon_reachable, platform_ok);
    let inventory = inventory_json(&plan.resolution);
    let selected_control = selected_control_json(plan, daemon_reachable);
    let candidates = service_candidates_json(plan);
    let requires_elevation = matches!(plan.scope, plan::ServiceScope::System) && !plan::is_admin();
    let broker = broker::broker_json(plan, daemon_reachable, platform_ok, requires_elevation);
    Ok(json!({
        "ok": overall_ok,
        "kind": "service_status",
        "state": service_state_name(daemon_reachable, platform_ok),
        "version": env!("CARGO_PKG_VERSION"),
        "user": current_user(),
        "resolution": inventory,
        "selected_control": selected_control,
        "candidates": candidates,
        "broker": broker,
        "requires_elevation": requires_elevation,
        "next_action": service_next_action(daemon_reachable, platform_ok),
        "scope": service_scope_name(plan.scope),
        "requested_scope": cli_service_scope_name(plan.requested_scope),
        "paths": {
            "config": plan.config_path,
            "route_store": plan.route_store_path,
            "source_exe": plan.source_exe,
            "installed_exe": plan.exe,
            "copy_exe": plan.copy_exe,
        },
        "control": {
            "endpoint": plan.endpoint,
        },
        "transport": {
            "plain_tcp": plan.transport.map(|addr| addr.to_string()),
            "tls_tcp": plan.tls_transport.map(|addr| addr.to_string()),
            "quic": plan.quic_transport.map(|addr| addr.to_string()),
        },
        "auth": {
            "token": plan.token.is_some(),
            "tls_cert": plan.tls_cert.is_some(),
            "tls_key": plan.tls_key.is_some(),
            "tls_client_ca": plan.tls_client_ca.is_some(),
        },
        "report_to": plan.report_to,
        "health": health,
        "daemon": daemon,
        "manager": manager,
        "platform": {
            "service_name": platform_service_name(plan.scope),
            "status": platform,
        }
    }))
}

fn service_manager_summary(plan: &ServicePlan, daemon_reachable: bool, platform_ok: bool) -> Value {
    let fallback_recommended = !daemon_reachable;
    json!({
        "kind": persistent_manager_kind(plan.scope),
        "service_name": platform_service_name(plan.scope),
        "requested_scope": cli_service_scope_name(plan.requested_scope),
        "selected_scope": service_scope_name(plan.scope),
        "selected_reason": plan.resolution.selected_reason,
        "resolution_next_action": plan.resolution.next_action.as_str(),
        "fallback_chain": plan.resolution.fallback_chain.iter().map(|scope| service_scope_name(*scope)).collect::<Vec<_>>(),
        "persistent_installed_or_registered": platform_ok,
        "daemon_reachable": daemon_reachable,
        "session_daemon_fallback": {
            "supported": true,
            "recommended": fallback_recommended,
            "reason": if fallback_recommended {
                "default daemon endpoint is not reachable; clients may start a session-owned daemon without installing a persistent service"
            } else {
                "default daemon endpoint is reachable; reuse the existing daemon"
            },
        },
        "next_action": service_next_action(daemon_reachable, platform_ok),
    })
}

fn selected_control_json(plan: &ServicePlan, daemon_reachable: bool) -> Value {
    let selected = if daemon_reachable {
        json!({
            "endpoint": plan.endpoint,
            "source": "configured_or_default",
            "reachable": true,
            "kind": control_endpoint_kind_from_str(&plan.endpoint),
        })
    } else {
        Value::Null
    };
    json!({
        "selected": selected,
        "preferred_order": [
            "system_service",
            "user_service",
            "configured_endpoint",
            "default_endpoint",
            "tcp_legacy"
        ],
        "configured_endpoint": plan.endpoint,
        "default_endpoint": crate::control_socket::default_endpoint_string(),
    })
}

fn service_candidates_json(plan: &ServicePlan) -> Value {
    let mut candidates = Vec::new();
    for probe in &plan.resolution.probe_chain {
        candidates.push(json!({
            "scope": service_scope_name(probe.scope),
            "service_name": probe.service_name,
            "exists": probe.exists,
            "healthy": probe.healthy,
            "accessible": probe.accessible,
            "permission_denied": probe.permission_denied,
            "control_endpoint": if probe.scope == plan.scope {
                Value::String(plan.endpoint.clone())
            } else {
                Value::Null
            },
            "version": Value::Null,
            "binary_path": Value::Null,
            "details": probe.details.clone(),
        }));
    }
    candidates.push(json!({
        "scope": "configured",
        "service_name": "configured_endpoint",
        "exists": true,
        "healthy": false,
        "accessible": true,
        "permission_denied": false,
        "control_endpoint": plan.endpoint,
        "version": env!("CARGO_PKG_VERSION"),
        "binary_path": plan.exe.clone(),
        "details": {
            "kind": control_endpoint_kind_from_str(&plan.endpoint),
        },
    }));
    Value::Array(candidates)
}

fn control_endpoint_kind_from_str(endpoint: &str) -> &'static str {
    if endpoint.starts_with("npipe://") {
        "named-pipe"
    } else if endpoint.starts_with("unix://") {
        "unix"
    } else if endpoint.starts_with("tcp://") {
        "tcp"
    } else {
        "tcp"
    }
}

fn service_state_name(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    match (daemon_reachable, platform_ok) {
        (true, true) => "running_with_persistent_manager",
        (true, false) => "running_without_persistent_manager",
        (false, true) => "persistent_manager_registered_but_daemon_unreachable",
        (false, false) => "unavailable",
    }
}

fn service_next_action(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    match (daemon_reachable, platform_ok) {
        (true, _) => "reuse_default_daemon",
        (false, true) => "start_or_repair_persistent_service",
        (false, false) => "install_persistent_service_or_start_session_daemon",
    }
}

fn persistent_manager_kind(scope: plan::ServiceScope) -> &'static str {
    match scope {
        plan::ServiceScope::User => {
            if cfg!(windows) {
                "windows_scheduled_task_user"
            } else if cfg!(target_os = "macos") {
                "launchd_user"
            } else if cfg!(target_os = "linux") {
                "systemd_user"
            } else {
                "user_service"
            }
        }
        plan::ServiceScope::System => {
            if cfg!(windows) {
                "windows_service_system"
            } else if cfg!(target_os = "macos") {
                "launchd_system"
            } else if cfg!(target_os = "linux") {
                "systemd_system"
            } else {
                "system_service"
            }
        }
    }
}

async fn service_health(plan: &ServicePlan) -> Value {
    let (config_health, config_snapshot) = config_file_health_with_snapshot(&plan.config_path);
    json!({
        "config": config_health,
        "route_store": route_store_health(&plan.route_store_path),
        "binary": binary_health(plan),
        "listeners": {
            "control": control_endpoint_health(&plan.endpoint),
            "plain_tcp": tcp_listener_health(plan.transport).await,
            "tls_tcp": tcp_listener_health(plan.tls_transport).await,
            "quic": quic_listener_health(plan.quic_transport),
        },
        "peers": peer_registry_health(config_snapshot.as_ref()).await,
    })
}

fn config_file_health_with_snapshot(path: &Path) -> (Value, Option<config::AppConfig>) {
    if !path.exists() {
        return (
            json!({
            "ok": false,
            "exists": false,
            "path": path,
            "message": "config file does not exist; run `ssh_proxy config init` or `ssh_proxy service install`"
            }),
            None,
        );
    }
    match std::fs::read_to_string(path).map_err(anyhow::Error::from) {
        Ok(text) => match toml::from_str::<config::AppConfig>(&text)
            .map_err(anyhow::Error::from)
            .and_then(|config| {
                config.validate_schema()?;
                Ok(config)
            }) {
            Ok(config) => {
                let raw_schema = toml::from_str::<toml::Value>(&text).ok().and_then(|value| {
                    value
                        .get("schema_version")
                        .and_then(|value| value.as_integer())
                });
                let health = json!({
                        "ok": true,
                        "exists": true,
                        "path": path,
                        "schema_version": config.schema_version,
                        "current_schema_version": config::CONFIG_SCHEMA_VERSION,
                        "schema_recorded": raw_schema.is_some(),
                        "legacy_without_schema": raw_schema.is_none(),
                });
                (health, Some(config))
            }
            Err(err) => (
                json!({
                    "ok": false,
                    "exists": true,
                    "path": path,
                    "current_schema_version": config::CONFIG_SCHEMA_VERSION,
                    "error": err.to_string(),
                }),
                None,
            ),
        },
        Err(err) => (
            json!({
                "ok": false,
                "exists": path.exists(),
                "path": path,
                "error": err.to_string(),
            }),
            None,
        ),
    }
}

fn route_store_health(path: &Path) -> Value {
    if !path.exists() {
        return json!({
            "ok": true,
            "exists": false,
            "path": path,
            "routes": 0,
            "message": "route store does not exist yet"
        });
    }
    match std::fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| serde_json::from_str::<Value>(&text).map_err(anyhow::Error::from))
    {
        Ok(value) => {
            let version = value.get("version").and_then(Value::as_u64);
            let routes_array = value.get("routes").and_then(Value::as_array);
            let routes = routes_array.map(Vec::len);
            let duplicate_ids = duplicate_route_ids(routes_array);
            let ok = version == Some(1) && routes.is_some() && duplicate_ids.is_empty();
            json!({
                "ok": ok,
                "exists": true,
                "path": path,
                "version": version,
                "current_version": 1,
                "routes": routes,
                "duplicate_ids": duplicate_ids,
            })
        }
        Err(err) => json!({
            "ok": false,
            "exists": true,
            "path": path,
            "error": err.to_string(),
        }),
    }
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

fn binary_health(plan: &ServicePlan) -> Value {
    json!({
        "source_exists": plan.source_exe.exists(),
        "installed_exists": plan.exe.exists(),
        "copy_exe": plan.copy_exe,
        "same_path": plan.source_exe == plan.exe,
    })
}

async fn peer_registry_health(config: Option<&config::AppConfig>) -> Value {
    let Some(config) = config else {
        return json!({
            "ok": false,
            "count": 0,
            "error": "config unavailable",
            "peers": [],
        });
    };
    let mut peers = config.peers.iter().collect::<Vec<_>>();
    peers.sort_by(|(left, _), (right, _)| left.cmp(right));
    let mut summaries = Vec::with_capacity(peers.len());
    let mut ok = true;
    for (alias, peer) in peers {
        let summary = peer_health(alias, peer).await;
        ok &= summary.get("ok").and_then(Value::as_bool).unwrap_or(false);
        summaries.push(summary);
    }
    json!({
        "ok": ok,
        "count": summaries.len(),
        "peers": summaries,
    })
}

async fn peer_health(alias: &str, peer: &config::PeerRecord) -> Value {
    let control = optional_control_endpoint_health(peer.control_endpoint.as_deref());
    let plain_tcp = tcp_listener_health(peer.transport).await;
    let tls_tcp = tcp_listener_health(peer.tls_transport).await;
    let quic = quic_listener_health(peer.quic_transport);
    let compatibility = peer_health::compatibility(peer);
    let ok = control.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && plain_tcp
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        && tls_tcp.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && quic.get("ok").and_then(Value::as_bool).unwrap_or(false)
        && compatibility
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    json!({
        "ok": ok,
        "alias": alias,
        "node_id": peer.node_id,
        "node_name": peer.node_name,
        "version": peer.version,
        "control_api_version": peer.control_api_version,
        "peer_protocol_version": peer.peer_protocol_version,
        "features": peer.features,
        "os": peer.os,
        "arch": peer.arch,
        "target": peer.target,
        "trust": peer.trust,
        "protocols": peer.known_transport_protocols(),
        "auth": {
            "token": peer.token.is_some(),
            "token_metadata": peer.token_metadata,
        },
        "compatibility": compatibility,
        "last_seen_unix": peer.last_seen_unix,
        "endpoints": {
            "control": control,
            "plain_tcp": plain_tcp,
            "tls_tcp": tls_tcp,
            "quic": quic,
        }
    })
}

fn control_endpoint_health(endpoint: &str) -> Value {
    match crate::control_socket::ControlEndpoint::parse(endpoint) {
        Ok(parsed) => json!({
            "ok": true,
            "endpoint": endpoint,
            "kind": control_endpoint_kind(&parsed),
        }),
        Err(err) => json!({
            "ok": false,
            "endpoint": endpoint,
            "error": err.to_string(),
        }),
    }
}

fn optional_control_endpoint_health(endpoint: Option<&str>) -> Value {
    match endpoint {
        Some(endpoint) => control_endpoint_health(endpoint),
        None => json!({
            "ok": false,
            "configured": false,
            "reachable": Value::Null,
            "message": "peer control endpoint is not recorded",
        }),
    }
}

fn control_endpoint_kind(endpoint: &crate::control_socket::ControlEndpoint) -> &'static str {
    match endpoint {
        crate::control_socket::ControlEndpoint::Tcp(_) => "tcp",
        #[cfg(unix)]
        crate::control_socket::ControlEndpoint::Unix(_) => "unix",
        #[cfg(windows)]
        crate::control_socket::ControlEndpoint::NamedPipe(_) => "named-pipe",
    }
}

async fn tcp_listener_health(addr: Option<SocketAddr>) -> Value {
    let Some(addr) = addr else {
        return json!({
            "ok": true,
            "configured": false,
            "reachable": Value::Null,
        });
    };
    let probe_addr = local_probe_addr(addr);
    match time::timeout(Duration::from_millis(500), TcpStream::connect(probe_addr)).await {
        Ok(Ok(stream)) => {
            drop(stream);
            json!({
                "ok": true,
                "configured": true,
                "addr": addr.to_string(),
                "probe_addr": probe_addr.to_string(),
                "reachable": true,
            })
        }
        Ok(Err(err)) => json!({
            "ok": false,
            "configured": true,
            "addr": addr.to_string(),
            "probe_addr": probe_addr.to_string(),
            "reachable": false,
            "error": err.to_string(),
        }),
        Err(_) => json!({
            "ok": false,
            "configured": true,
            "addr": addr.to_string(),
            "probe_addr": probe_addr.to_string(),
            "reachable": false,
            "error": "TCP listener probe timed out after 500 ms",
        }),
    }
}

fn quic_listener_health(addr: Option<SocketAddr>) -> Value {
    match addr {
        Some(addr) => json!({
            "ok": true,
            "configured": true,
            "addr": addr.to_string(),
            "reachable": Value::Null,
            "message": "QUIC UDP listener reachability is not probed by service status yet",
        }),
        None => json!({
            "ok": true,
            "configured": false,
            "reachable": Value::Null,
        }),
    }
}

fn local_probe_addr(addr: SocketAddr) -> SocketAddr {
    if !addr.ip().is_unspecified() {
        return addr;
    }
    match addr.ip() {
        IpAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port()),
        IpAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port()),
    }
}

async fn query_daemon_status(plan: &ServicePlan) -> Value {
    let endpoint = match crate::control_socket::ControlEndpoint::parse(&plan.endpoint) {
        Ok(endpoint) => endpoint,
        Err(err) => {
            return json!({
                "reachable": false,
                "error": err.to_string(),
            });
        }
    };
    let request = match crate::node_daemon::NodeRequest::command("status")
        .with_auth_token(plan.token.as_deref())
        .to_line()
    {
        Ok(request) => request,
        Err(err) => {
            return json!({
                "reachable": false,
                "error": err.to_string(),
            });
        }
    };
    match time::timeout(
        Duration::from_secs(2),
        crate::control_socket::request(&endpoint, &request),
    )
    .await
    {
        Ok(Ok(response)) => match serde_json::from_str::<Value>(&response) {
            Ok(mut value) => {
                redact_daemon_status(&mut value);
                json!({
                    "reachable": true,
                    "status": value,
                })
            }
            Err(err) => json!({
                "reachable": true,
                "error": format!("daemon status was not JSON: {err}"),
            }),
        },
        Ok(Err(err)) => json!({
            "reachable": false,
            "error": err.to_string(),
        }),
        Err(_) => json!({
            "reachable": false,
            "error": "daemon status request timed out after 2 seconds",
        }),
    }
}

fn redact_daemon_status(value: &mut Value) {
    if let Some(auth) = value.get_mut("auth").and_then(Value::as_object_mut) {
        auth.remove("token");
    }
}

fn current_user() -> String {
    whoami::username().unwrap_or_else(|_| "unknown".to_string())
}

fn service_scope_name(scope: plan::ServiceScope) -> &'static str {
    match scope {
        plan::ServiceScope::User => "user",
        plan::ServiceScope::System => "system",
    }
}

fn cli_service_scope_name(scope: cli::ServiceScope) -> &'static str {
    match scope {
        cli::ServiceScope::Auto => "auto",
        cli::ServiceScope::User => "user",
        cli::ServiceScope::System => "system",
    }
}

fn platform_service_name(scope: plan::ServiceScope) -> String {
    plan::platform_service_name(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_plan() -> ServicePlan {
        ServicePlan::new(
            cli::ServiceArgs {
                scope: cli::ServiceScope::User,
                control: Some("tcp://127.0.0.1:1".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                no_transport: false,
                token: Some("secret".to_string()),
                tls_transport: None,
                quic_transport: Some("127.0.0.1:19083".parse().unwrap()),
                tls_cert: None,
                tls_key: None,
                tls_client_ca: None,
                report_to: vec!["tcp://127.0.0.1:19091".to_string()],
                install_dir: None,
                no_copy: true,
                json: false,
                elevate: false,
                command: cli::ServiceCommand::Status,
            },
            config::AppConfig::default(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn service_status_summary_is_redacted_and_structured() {
        let plan = status_plan();
        let summary = service_status_summary(&plan).await.unwrap();

        assert_eq!(summary["kind"], "service_status");
        assert_eq!(summary["scope"], "user");
        assert_eq!(summary["auth"]["token"], true);
        assert_eq!(summary["transport"]["plain_tcp"], "127.0.0.1:19080");
        assert_eq!(summary["transport"]["quic"], "127.0.0.1:19083");
        assert_eq!(summary["health"]["listeners"]["control"]["ok"], true);
        assert_eq!(summary["health"]["route_store"]["ok"], true);
        assert_eq!(summary["health"]["listeners"]["quic"]["configured"], true);
        assert!(summary["state"].is_string());
        assert_eq!(
            summary["manager"]["session_daemon_fallback"]["supported"],
            true
        );
        assert!(summary["manager"]["next_action"].is_string());
        assert!(summary["platform"]["status"]["ok"].is_boolean());
        assert!(!summary.to_string().contains("secret"));
        assert!(summary["daemon"]["reachable"].is_boolean());
    }

    #[test]
    fn service_state_names_cover_core_cases() {
        assert_eq!(
            service_state_name(true, true),
            "running_with_persistent_manager"
        );
        assert_eq!(
            service_state_name(true, false),
            "running_without_persistent_manager"
        );
        assert_eq!(
            service_state_name(false, true),
            "persistent_manager_registered_but_daemon_unreachable"
        );
        assert_eq!(service_state_name(false, false), "unavailable");
        assert_eq!(service_next_action(true, false), "reuse_default_daemon");
        assert_eq!(
            service_next_action(false, true),
            "start_or_repair_persistent_service"
        );
        assert_eq!(
            service_next_action(false, false),
            "install_persistent_service_or_start_session_daemon"
        );
    }

    #[test]
    fn route_store_health_reports_invalid_json() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-invalid-route-store-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "{not-json").unwrap();

        let health = route_store_health(&path);

        assert_eq!(health["ok"], false);
        assert_eq!(health["exists"], true);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn route_store_health_reports_duplicate_ids() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-duplicate-route-store-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"version":1,"routes":[{"id":"same"},{"id":"same"}]}"#,
        )
        .unwrap();

        let health = route_store_health(&path);

        assert_eq!(health["ok"], false);
        assert_eq!(health["duplicate_ids"][0], "same");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn config_file_health_reports_future_schema() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-future-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = 999\n").unwrap();

        let (health, _) = config_file_health_with_snapshot(&path);

        assert_eq!(health["ok"], false);
        assert!(
            health["error"]
                .as_str()
                .expect("error")
                .contains("newer than this binary supports")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unspecified_listener_probe_uses_loopback() {
        let addr = "0.0.0.0:19080".parse().unwrap();
        let probe = local_probe_addr(addr);

        assert_eq!(probe.to_string(), "127.0.0.1:19080");
    }

    #[tokio::test]
    async fn peer_registry_health_is_sorted_and_redacted() {
        let path =
            std::env::temp_dir().join(format!("ssh_proxy-peer-health-{}.toml", std::process::id()));
        let mut config = config::AppConfig::default();
        config.peers.insert(
            "zeta".to_string(),
            config::PeerRecord {
                node_id: Some("node-z".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                control_api_version: Some(crate::node_daemon::control_api_version()),
                peer_protocol_version: Some(crate::node_daemon::peer_protocol_version()),
                features: crate::node_daemon::peer_protocol_features(),
                control_endpoint: Some("tcp://127.0.0.1:1".to_string()),
                token: Some("peer-secret".to_string()),
                ..Default::default()
            },
        );
        config.peers.insert(
            "alpha".to_string(),
            config::PeerRecord {
                node_name: Some("node-a".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                control_api_version: Some(crate::node_daemon::control_api_version()),
                peer_protocol_version: Some(crate::node_daemon::peer_protocol_version()),
                features: crate::node_daemon::peer_protocol_features(),
                control_endpoint: Some("tcp://127.0.0.1:1".to_string()),
                transport_protocols: vec!["plain-tcp".to_string()],
                ..Default::default()
            },
        );
        std::fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let (_, config) = config_file_health_with_snapshot(&path);
        let health = peer_registry_health(config.as_ref()).await;

        assert_eq!(health["ok"], true);
        assert_eq!(health["count"], 2);
        assert_eq!(health["peers"][0]["alias"], "alpha");
        assert_eq!(health["peers"][0]["compatibility"]["ok"], true);
        assert_eq!(health["peers"][1]["alias"], "zeta");
        assert_eq!(health["peers"][1]["auth"]["token"], true);
        assert!(!health.to_string().contains("peer-secret"));
        let _ = std::fs::remove_file(path);
    }
}
