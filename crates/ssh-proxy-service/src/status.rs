use serde_json::{Value, json};

use crate::{RequestedServiceScope, ServiceNextAction, ServiceProbeSummary, ServiceScope};

#[derive(Debug, Clone, Copy)]
pub struct ServiceManagerSummaryInput<'a> {
    pub kind: &'a str,
    pub lifecycle_provider: &'a str,
    pub lifecycle_role: &'a str,
    pub service_name: &'a str,
    pub requested_scope: RequestedServiceScope,
    pub selected_scope: ServiceScope,
    pub selected_reason: &'a str,
    pub resolution_next_action: ServiceNextAction,
    pub fallback_chain: &'a [ServiceScope],
    pub persistent_installed_or_registered: bool,
    pub daemon_reachable: bool,
}

pub fn service_manager_summary(input: ServiceManagerSummaryInput<'_>) -> Value {
    let fallback_recommended = !input.daemon_reachable;
    json!({
        "kind": input.kind,
        "lifecycle_provider": input.lifecycle_provider,
        "lifecycle_role": input.lifecycle_role,
        "service_name": input.service_name,
        "requested_scope": input.requested_scope.as_str(),
        "selected_scope": input.selected_scope.as_str(),
        "selected_reason": input.selected_reason,
        "resolution_next_action": input.resolution_next_action.as_str(),
        "fallback_chain": input.fallback_chain.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
        "persistent_installed_or_registered": input.persistent_installed_or_registered,
        "daemon_reachable": input.daemon_reachable,
        "session_daemon_fallback": {
            "supported": true,
            "recommended": fallback_recommended,
            "reason": session_daemon_fallback_reason(fallback_recommended),
        },
        "next_action": service_next_action(input.daemon_reachable, input.persistent_installed_or_registered),
    })
}

pub fn selected_control_summary(
    endpoint: &str,
    default_endpoint: &str,
    daemon_reachable: bool,
) -> Value {
    let selected = if daemon_reachable {
        json!({
            "endpoint": endpoint,
            "source": "configured_or_default",
            "reachable": true,
            "kind": control_endpoint_kind_from_str(endpoint),
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
        "configured_endpoint": endpoint,
        "default_endpoint": default_endpoint,
    })
}

pub fn service_candidates_summary(
    probe_chain: &[ServiceProbeSummary],
    selected_scope: ServiceScope,
    endpoint: &str,
    version: &str,
    binary_path: &str,
) -> Value {
    let mut candidates = Vec::new();
    for probe in probe_chain {
        candidates.push(json!({
            "scope": probe.scope.as_str(),
            "service_name": probe.service_name,
            "exists": probe.exists,
            "healthy": probe.healthy,
            "accessible": probe.accessible,
            "permission_denied": probe.permission_denied,
            "control_endpoint": if probe.scope == selected_scope {
                Value::String(endpoint.to_string())
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
        "control_endpoint": endpoint,
        "version": version,
        "binary_path": binary_path,
        "details": {
            "kind": control_endpoint_kind_from_str(endpoint),
        },
    }));
    Value::Array(candidates)
}

pub fn service_state_name(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    match (daemon_reachable, platform_ok) {
        (true, true) => "running_with_persistent_manager",
        (true, false) => "running_without_persistent_manager",
        (false, true) => "persistent_manager_registered_but_daemon_unreachable",
        (false, false) => "unavailable",
    }
}

pub fn service_next_action(daemon_reachable: bool, platform_ok: bool) -> &'static str {
    match (daemon_reachable, platform_ok) {
        (true, _) => "reuse_default_daemon",
        (false, true) => "start_or_repair_persistent_service",
        (false, false) => "install_persistent_service_or_start_session_daemon",
    }
}

pub fn persistent_manager_kind(scope: ServiceScope) -> &'static str {
    match scope {
        ServiceScope::User => {
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
        ServiceScope::System => {
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

pub fn control_endpoint_kind_from_str(endpoint: &str) -> &'static str {
    if endpoint.starts_with("npipe://") {
        "named-pipe"
    } else if endpoint.starts_with("unix://") {
        "unix"
    } else {
        "tcp"
    }
}

fn session_daemon_fallback_reason(fallback_recommended: bool) -> &'static str {
    if fallback_recommended {
        "default daemon endpoint is not reachable; clients may start a session-owned daemon without installing a persistent service"
    } else {
        "default daemon endpoint is reachable; reuse the existing daemon"
    }
}

#[cfg(test)]
mod tests {
    use crate::{ServiceProbeState, service_probe_summary};

    use super::*;

    #[test]
    fn service_status_state_and_next_action_cover_core_cases() {
        assert_eq!(
            service_state_name(true, true),
            "running_with_persistent_manager"
        );
        assert_eq!(service_state_name(false, false), "unavailable");
        assert_eq!(service_next_action(true, false), "reuse_default_daemon");
        assert_eq!(
            service_next_action(false, true),
            "start_or_repair_persistent_service"
        );
    }

    #[test]
    fn manager_summary_classifies_session_fallback() {
        let summary = service_manager_summary(ServiceManagerSummaryInput {
            kind: "systemd_user",
            lifecycle_provider: "systemd_user",
            lifecycle_role: "local_daemon",
            service_name: "ssh_proxy",
            requested_scope: RequestedServiceScope::Auto,
            selected_scope: ServiceScope::User,
            selected_reason: "selected existing healthy User service",
            resolution_next_action: ServiceNextAction::Reuse,
            fallback_chain: &[ServiceScope::System, ServiceScope::User],
            persistent_installed_or_registered: true,
            daemon_reachable: false,
        });

        assert_eq!(summary["kind"], "systemd_user");
        assert_eq!(summary["session_daemon_fallback"]["recommended"], true);
        assert_eq!(summary["next_action"], "start_or_repair_persistent_service");
    }

    #[test]
    fn selected_control_and_candidates_preserve_status_shape() {
        let selected =
            selected_control_summary("tcp://127.0.0.1:19081", "tcp://127.0.0.1:19081", true);
        let probe = service_probe_summary(
            ServiceScope::User,
            "ssh_proxy".to_string(),
            ServiceProbeState::Healthy,
            true,
            true,
            true,
            false,
            json!({}),
        );
        let candidates = service_candidates_summary(
            &[probe],
            ServiceScope::User,
            "tcp://127.0.0.1:19081",
            "1.2.3",
            "/tmp/ssh_proxy",
        );

        assert_eq!(selected["selected"]["kind"], "tcp");
        assert_eq!(candidates[0]["control_endpoint"], "tcp://127.0.0.1:19081");
        assert_eq!(candidates[1]["scope"], "configured");
    }
}
