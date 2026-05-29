use serde_json::{Value, json};

use crate::{ServiceProbeSummary, ServiceScope};

#[derive(Debug, Clone, Copy)]
pub struct ServiceBrokerReportInput<'a> {
    pub scope: ServiceScope,
    pub endpoint: &'a str,
    pub default_endpoint: &'a str,
    pub system_endpoint: &'a str,
    pub daemon_reachable: bool,
    pub platform_ok: bool,
    pub requires_elevation: bool,
    pub current_user_is_admin: bool,
    pub probe_chain: &'a [ServiceProbeSummary],
}

pub fn service_broker_report(input: ServiceBrokerReportInput<'_>) -> Value {
    let candidates = broker_candidates(input);
    let selected = select_broker_candidate(&candidates);
    json!({
        "selected": selected,
        "candidates": candidates,
        "capabilities": broker_capabilities(input.scope),
        "next_action": broker_next_action(input),
        "permission_boundary": permission_boundary(input.scope, input.requires_elevation),
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

fn broker_candidates(input: ServiceBrokerReportInput<'_>) -> Vec<Value> {
    let mut candidates = Vec::new();
    candidates.push(platform_candidate(
        "system_broker",
        ServiceScope::System,
        input.system_endpoint,
        input,
    ));
    candidates.push(platform_candidate(
        "user_broker",
        ServiceScope::User,
        input.default_endpoint,
        input,
    ));

    if input.endpoint != input.default_endpoint && input.endpoint != input.system_endpoint {
        candidates.push(endpoint_candidate(
            "configured_endpoint",
            input.endpoint,
            input.daemon_reachable,
            true,
            false,
            "configured daemon endpoint",
        ));
    }

    candidates.push(endpoint_candidate(
        "default_endpoint",
        input.default_endpoint,
        input.daemon_reachable,
        input.endpoint == input.default_endpoint,
        false,
        "default user-local daemon endpoint",
    ));
    candidates.push(endpoint_candidate(
        "tcp_legacy",
        "tcp://127.0.0.1:19081",
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
    endpoint: &str,
    input: ServiceBrokerReportInput<'_>,
) -> Value {
    let probe = input.probe_chain.iter().find(|probe| probe.scope == scope);
    let selected_plan_scope = input.scope == scope;
    let reachable = selected_plan_scope && input.daemon_reachable;
    json!({
        "source": source,
        "scope": scope.as_str(),
        "endpoint": endpoint,
        "kind": endpoint_kind(endpoint),
        "reachable": reachable,
        "healthy": reachable || (selected_plan_scope && input.platform_ok),
        "registered": probe.is_some_and(|probe| probe.exists),
        "accessible": probe.is_none_or(|probe| probe.accessible),
        "permission_denied": probe.is_some_and(|probe| probe.permission_denied),
        "requires_token": endpoint.starts_with("tcp://"),
        "selected_by_current_plan": selected_plan_scope,
        "permission_boundary": permission_boundary(
            scope,
            matches!(scope, ServiceScope::System) && !input.current_user_is_admin,
        ),
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

fn broker_next_action(input: ServiceBrokerReportInput<'_>) -> &'static str {
    if input.daemon_reachable {
        "reuse_broker"
    } else if input.platform_ok {
        "start_or_repair_broker"
    } else if input.requires_elevation {
        "install_system_elevated_or_user_broker"
    } else if input
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

fn endpoint_kind(endpoint: &str) -> &'static str {
    if endpoint.starts_with("npipe://") {
        "named-pipe"
    } else if endpoint.starts_with("unix://") {
        "unix"
    } else {
        "tcp"
    }
}

#[cfg(test)]
mod tests {
    use crate::{ServiceProbeState, service_probe_summary};

    use super::*;

    #[test]
    fn broker_report_prefers_reachable_current_control() {
        let report = service_broker_report(ServiceBrokerReportInput {
            scope: ServiceScope::User,
            endpoint: "tcp://127.0.0.1:1",
            default_endpoint: "tcp://127.0.0.1:1",
            system_endpoint: "unix:///run/ssh_proxy/control.sock",
            daemon_reachable: true,
            platform_ok: true,
            requires_elevation: false,
            current_user_is_admin: false,
            probe_chain: &[],
        });

        assert_eq!(report["next_action"], "reuse_broker");
        assert!(report["selected"]["reachable"].as_bool().unwrap());
        assert!(
            report["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("route_intent"))
        );
    }

    #[test]
    fn broker_report_marks_system_permission_boundary() {
        let probe = service_probe_summary(
            ServiceScope::System,
            "ssh_proxy".to_string(),
            ServiceProbeState::PermissionDenied,
            true,
            false,
            false,
            true,
            json!({}),
        );
        let report = service_broker_report(ServiceBrokerReportInput {
            scope: ServiceScope::System,
            endpoint: "npipe://ssh_proxy/system/control",
            default_endpoint: "npipe://ssh_proxy/user/control",
            system_endpoint: "npipe://ssh_proxy/system/control",
            daemon_reachable: false,
            platform_ok: false,
            requires_elevation: true,
            current_user_is_admin: false,
            probe_chain: &[probe],
        });

        assert_eq!(report["policy"]["arbitrary_shell"], false);
        assert_eq!(report["permission_boundary"]["arbitrary_shell"], false);
        assert!(report["next_action"].as_str().unwrap().contains("elevated"));
    }
}
