use anyhow::Result;
use serde_json::{Value, json};
use tokio::time::{self, Duration};

use super::{
    broker, health,
    inventory::inventory_json,
    labels::{cli_service_scope_name, platform_service_name, service_scope_name},
    plan::{self, ServicePlan},
    platform,
};

pub(super) async fn status_service(plan: &ServicePlan, json_output: bool) -> Result<()> {
    let summary = service_status_summary(plan).await?;
    if json_output {
        println!("{}", serde_json::to_string(&summary)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    }
    Ok(())
}

pub(super) async fn service_status_summary(plan: &ServicePlan) -> Result<Value> {
    let daemon = query_daemon_status(plan).await;
    let health = health::service_health(plan).await;
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

pub(super) fn service_state_name(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    match (daemon_reachable, platform_ok) {
        (true, true) => "running_with_persistent_manager",
        (true, false) => "running_without_persistent_manager",
        (false, true) => "persistent_manager_registered_but_daemon_unreachable",
        (false, false) => "unavailable",
    }
}

pub(super) fn service_next_action(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    match (daemon_reachable, platform_ok) {
        (true, _) => "reuse_default_daemon",
        (false, true) => "start_or_repair_persistent_service",
        (false, false) => "install_persistent_service_or_start_session_daemon",
    }
}

fn service_manager_summary(plan: &ServicePlan, daemon_reachable: bool, platform_ok: bool) -> Value {
    let fallback_recommended = !daemon_reachable;
    let lifecycle = plan.lifecycle_spec();
    json!({
        "kind": persistent_manager_kind(plan.scope),
        "lifecycle_provider": lifecycle.provider.manager_name(),
        "lifecycle_role": lifecycle.role,
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
    } else {
        "tcp"
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
