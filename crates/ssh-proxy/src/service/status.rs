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
use crate::peer_lifecycle::spec::PeerLifecycleRole;

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
    ssh_proxy_service::service_state_name(daemon_reachable, platform_ok)
}

pub(super) fn service_next_action(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    ssh_proxy_service::service_next_action(daemon_reachable, platform_ok)
}

fn service_manager_summary(plan: &ServicePlan, daemon_reachable: bool, platform_ok: bool) -> Value {
    let lifecycle = plan.lifecycle_spec();
    ssh_proxy_service::service_manager_summary(ssh_proxy_service::ServiceManagerSummaryInput {
        kind: ssh_proxy_service::persistent_manager_kind(plan.scope),
        lifecycle_provider: lifecycle.provider.manager_name(),
        lifecycle_role: lifecycle_role_name(lifecycle.role),
        service_name: &platform_service_name(plan.scope),
        requested_scope: plan.resolution.requested_scope,
        selected_scope: plan.scope,
        selected_reason: &plan.resolution.selected_reason,
        resolution_next_action: plan.resolution.next_action,
        fallback_chain: &plan.resolution.fallback_chain,
        persistent_installed_or_registered: platform_ok,
        daemon_reachable,
    })
}

fn selected_control_json(plan: &ServicePlan, daemon_reachable: bool) -> Value {
    ssh_proxy_service::selected_control_summary(
        &plan.endpoint,
        &crate::control_socket::default_endpoint_string(),
        daemon_reachable,
    )
}

fn service_candidates_json(plan: &ServicePlan) -> Value {
    ssh_proxy_service::service_candidates_summary(
        &plan.resolution.probe_chain,
        plan.scope,
        &plan.endpoint,
        env!("CARGO_PKG_VERSION"),
        &plan.exe.display().to_string(),
    )
}

fn lifecycle_role_name(role: PeerLifecycleRole) -> &'static str {
    match role {
        PeerLifecycleRole::LocalDaemon => "local_daemon",
        PeerLifecycleRole::RemotePeer => "remote_peer",
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
