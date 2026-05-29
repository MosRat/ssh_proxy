use serde_json::{Value, json};

use super::plan::{ServicePlan, ServiceScope};

pub(super) fn broker_json(
    plan: &ServicePlan,
    daemon_reachable: bool,
    platform_ok: bool,
    requires_elevation: bool,
) -> Value {
    let candidates = broker_candidates(plan, daemon_reachable, platform_ok);
    let selected = select_broker_candidate(&candidates);
    json!({
        "selected": selected,
        "candidates": candidates,
        "capabilities": broker_capabilities(plan.scope),
        "next_action": broker_next_action(daemon_reachable, platform_ok, requires_elevation, plan),
        "permission_boundary": permission_boundary(plan.scope, requires_elevation),
        "policy": {
            "mode": "kernel-first",
            "openssh": "final-fallback",
            "arbitrary_shell": false,
            "control_preference": [
                "system_broker",
                "user_broker",
                "configured_endpoint",
                "default_endpoint",
                "tcp_legacy",
                "session_daemon"
            ],
        },
    })
}

fn broker_candidates(plan: &ServicePlan, daemon_reachable: bool, platform_ok: bool) -> Vec<Value> {
    let mut candidates = Vec::new();
    candidates.push(platform_candidate(
        "system_broker",
        ServiceScope::System,
        system_endpoint(),
        plan,
        daemon_reachable,
        platform_ok,
    ));
    candidates.push(platform_candidate(
        "user_broker",
        ServiceScope::User,
        crate::control_socket::default_endpoint_string(),
        plan,
        daemon_reachable,
        platform_ok,
    ));

    if plan.endpoint != crate::control_socket::default_endpoint_string()
        && plan.endpoint != system_endpoint()
    {
        candidates.push(endpoint_candidate(
            "configured_endpoint",
            &plan.endpoint,
            daemon_reachable,
            plan.endpoint == selected_plan_endpoint(plan),
            false,
            "configured daemon endpoint",
        ));
    }

    candidates.push(endpoint_candidate(
        "default_endpoint",
        &crate::control_socket::default_endpoint_string(),
        daemon_reachable,
        plan.endpoint == crate::control_socket::default_endpoint_string(),
        false,
        "default user-local daemon endpoint",
    ));
    candidates.push(endpoint_candidate(
        "tcp_legacy",
        &legacy_tcp_endpoint(),
        false,
        false,
        true,
        "legacy localhost TCP control endpoint",
    ));
    candidates
}

fn platform_candidate(
    source: &str,
    scope: ServiceScope,
    endpoint: String,
    plan: &ServicePlan,
    daemon_reachable: bool,
    platform_ok: bool,
) -> Value {
    let probe = plan
        .resolution
        .probe_chain
        .iter()
        .find(|probe| probe.scope == scope);
    let selected_plan_scope = plan.scope == scope;
    let reachable = selected_plan_scope && daemon_reachable;
    json!({
        "source": source,
        "scope": scope_name(scope),
        "endpoint": endpoint,
        "kind": endpoint_kind(&endpoint),
        "reachable": reachable,
        "healthy": reachable || (selected_plan_scope && platform_ok),
        "registered": probe.is_some_and(|probe| probe.exists),
        "accessible": probe.is_none_or(|probe| probe.accessible),
        "permission_denied": probe.is_some_and(|probe| probe.permission_denied),
        "requires_token": endpoint.starts_with("tcp://"),
        "selected_by_current_plan": selected_plan_scope,
        "permission_boundary": permission_boundary(scope, matches!(scope, ServiceScope::System) && !super::plan::is_admin()),
    })
}

fn endpoint_candidate(
    source: &str,
    endpoint: &str,
    daemon_reachable: bool,
    selected_by_current_plan: bool,
    legacy: bool,
    reason: &str,
) -> Value {
    json!({
        "source": source,
        "scope": Value::Null,
        "endpoint": endpoint,
        "kind": endpoint_kind(endpoint),
        "reachable": selected_by_current_plan && daemon_reachable,
        "healthy": selected_by_current_plan && daemon_reachable,
        "registered": false,
        "accessible": true,
        "permission_denied": false,
        "requires_token": endpoint.starts_with("tcp://"),
        "legacy": legacy,
        "selected_by_current_plan": selected_by_current_plan,
        "reason": reason,
    })
}

fn select_broker_candidate(candidates: &[Value]) -> Value {
    candidates
        .iter()
        .find(|candidate| candidate["reachable"].as_bool().unwrap_or(false))
        .or_else(|| {
            candidates.iter().find(|candidate| {
                candidate["registered"].as_bool().unwrap_or(false)
                    && candidate["accessible"].as_bool().unwrap_or(false)
                    && !candidate["permission_denied"].as_bool().unwrap_or(false)
            })
        })
        .or_else(|| {
            candidates
                .iter()
                .find(|candidate| candidate["source"] == "user_broker")
        })
        .cloned()
        .unwrap_or(Value::Null)
}

fn broker_next_action(
    daemon_reachable: bool,
    platform_ok: bool,
    requires_elevation: bool,
    plan: &ServicePlan,
) -> &'static str {
    if daemon_reachable {
        "reuse_broker"
    } else if platform_ok {
        "start_or_repair_broker"
    } else if requires_elevation {
        "install_system_elevated_or_user_broker"
    } else if plan
        .resolution
        .probe_chain
        .iter()
        .any(|probe| probe.permission_denied)
    {
        "session_daemon"
    } else {
        "install_user_broker_or_session_daemon"
    }
}

fn broker_capabilities(scope: ServiceScope) -> Vec<&'static str> {
    let mut capabilities = vec![
        "node_daemon_lifecycle",
        "route_intent",
        "route_readiness",
        "peer_descriptor_adoption",
        "peer_bootstrap",
        "peer_update_jobs",
        "session_daemon_fallback",
    ];
    if matches!(scope, ServiceScope::System) {
        capabilities.extend([
            "system_service_install",
            "system_service_update",
            "user_daemon_repair",
        ]);
    } else {
        capabilities.extend(["user_service_install", "user_service_update"]);
    }
    capabilities
}

fn permission_boundary(scope: ServiceScope, requires_elevation: bool) -> Value {
    match scope {
        ServiceScope::System => json!({
            "scope": "system",
            "requires_elevation": requires_elevation,
            "may_manage_system_service": true,
            "may_manage_user_daemons": true,
            "arbitrary_shell": false,
        }),
        ServiceScope::User => json!({
            "scope": "user",
            "requires_elevation": false,
            "may_manage_system_service": false,
            "may_manage_user_daemons": true,
            "arbitrary_shell": false,
        }),
    }
}

fn selected_plan_endpoint(plan: &ServicePlan) -> String {
    plan.endpoint.clone()
}

fn endpoint_kind(endpoint: &str) -> &'static str {
    if endpoint.starts_with("npipe://") {
        "named-pipe"
    } else if endpoint.starts_with("unix://") {
        "unix"
    } else {
        "tcp"
    }
}

fn scope_name(scope: ServiceScope) -> &'static str {
    match scope {
        ServiceScope::System => "system",
        ServiceScope::User => "user",
    }
}

fn system_endpoint() -> String {
    #[cfg(windows)]
    {
        "npipe://ssh_proxy/system/control".to_string()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "unix:///run/ssh_proxy/control.sock".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "unix:///var/run/ssh_proxy/control.sock".to_string()
    }
}

fn legacy_tcp_endpoint() -> String {
    "tcp://127.0.0.1:19081".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli, config};

    fn plan(scope: cli::ServiceScope) -> ServicePlan {
        ServicePlan::new(
            cli::ServiceArgs {
                scope,
                control: Some("tcp://127.0.0.1:1".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                no_transport: false,
                token: Some("secret".to_string()),
                tls_transport: None,
                quic_transport: None,
                tls_cert: None,
                tls_key: None,
                tls_client_ca: None,
                report_to: vec![],
                install_dir: None,
                no_copy: true,
                json: true,
                elevate: false,
                command: cli::ServiceCommand::Status,
            },
            config::AppConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn broker_json_prefers_reachable_current_control() {
        let value = broker_json(&plan(cli::ServiceScope::User), true, true, false);

        assert_eq!(value["next_action"], "reuse_broker");
        assert!(value["selected"]["reachable"].as_bool().unwrap());
        assert!(
            value["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("route_intent"))
        );
    }

    #[test]
    fn broker_json_never_grants_arbitrary_shell() {
        let value = broker_json(&plan(cli::ServiceScope::System), false, false, true);

        assert_eq!(value["policy"]["arbitrary_shell"], false);
        assert_eq!(value["permission_boundary"]["arbitrary_shell"], false);
        assert!(value["next_action"].as_str().unwrap().contains("elevated"));
    }
}
