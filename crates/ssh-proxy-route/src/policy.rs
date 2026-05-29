use serde::{Deserialize, Serialize};
use ssh_proxy_core::model::WorkloadHint;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePoolSizingInput {
    pub has_tcp_target: bool,
    pub command_transport_pool_size: Option<usize>,
    pub profile_transport_pool_size: Option<usize>,
    pub default_transport_pool_size: Option<usize>,
    pub command_ssh_session_pool_size: Option<usize>,
    pub profile_ssh_session_pool_size: Option<usize>,
    pub default_ssh_session_pool_size: Option<usize>,
    pub command_workload_hint: Option<WorkloadHint>,
    pub profile_workload_hint: Option<WorkloadHint>,
    pub default_workload_hint: Option<WorkloadHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTransportPoolPolicy {
    pub size: usize,
    pub source: String,
    pub reason: String,
    pub pool_policy: String,
    pub workload_hint: WorkloadHint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSshSessionPoolPolicy {
    pub size: usize,
    pub source: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub fn plan_transport_pool(input: &RoutePoolSizingInput) -> RouteTransportPoolPolicy {
    if let Some(value) = input.command_transport_pool_size {
        let size = value.max(1);
        return RouteTransportPoolPolicy {
            size,
            source: "command-line".to_string(),
            reason: pool_reason("--transport-pool-size", value, size),
            pool_policy: "explicit".to_string(),
            workload_hint: workload_hint_policy(input),
        };
    }
    if let Some(value) = input.profile_transport_pool_size {
        let size = value.max(1);
        return RouteTransportPoolPolicy {
            size,
            source: "profile".to_string(),
            reason: pool_reason("target profile transport_pool_size", value, size),
            pool_policy: "explicit".to_string(),
            workload_hint: workload_hint_policy(input),
        };
    }
    if let Some(value) = input.default_transport_pool_size {
        let size = value.max(1);
        return RouteTransportPoolPolicy {
            size,
            source: "defaults".to_string(),
            reason: pool_reason("[defaults].transport_pool_size", value, size),
            pool_policy: "explicit".to_string(),
            workload_hint: workload_hint_policy(input),
        };
    }
    let hint = workload_hint_policy(input);
    RouteTransportPoolPolicy {
        size: implicit_transport_pool_size(input, hint),
        source: "implicit".to_string(),
        reason: implicit_transport_pool_reason(input, hint),
        pool_policy: pool_policy_name(hint).to_string(),
        workload_hint: hint,
    }
}

pub fn plan_ssh_session_pool(input: &RoutePoolSizingInput) -> RouteSshSessionPoolPolicy {
    if let Some(value) = input.command_ssh_session_pool_size {
        let size = value.max(1);
        return RouteSshSessionPoolPolicy {
            size,
            source: "command-line".to_string(),
            reason: pool_reason("--ssh-session-pool-size", value, size),
            warning: ssh_session_pool_warning(size),
        };
    }
    if let Some(value) = input.profile_ssh_session_pool_size {
        let size = value.max(1);
        return RouteSshSessionPoolPolicy {
            size,
            source: "profile".to_string(),
            reason: pool_reason("target profile ssh_session_pool_size", value, size),
            warning: ssh_session_pool_warning(size),
        };
    }
    if let Some(value) = input.default_ssh_session_pool_size {
        let requested = value.max(1);
        let size = requested.min(2);
        return RouteSshSessionPoolPolicy {
            size,
            source: "defaults".to_string(),
            reason: if requested == size {
                pool_reason("[defaults].ssh_session_pool_size", value, size)
            } else {
                format!(
                    "loaded from [defaults].ssh_session_pool_size={value}; capped to pool=2 because only command-line/profile benchmark experiments may exceed the implicit-safe ssh-native range"
                )
            },
            warning: if requested > size {
                Some(
                    "ssh-native defaults above 2 are not auto-selected; use --ssh-session-pool-size or a target profile for explicit benchmark experiments"
                        .to_string(),
                )
            } else {
                ssh_session_pool_warning(size)
            },
        };
    }

    let size = implicit_ssh_session_pool_size(input);
    RouteSshSessionPoolPolicy {
        size,
        source: "implicit".to_string(),
        reason: implicit_ssh_session_pool_reason(input),
        warning: None,
    }
}

fn workload_hint_policy(input: &RoutePoolSizingInput) -> WorkloadHint {
    input
        .command_workload_hint
        .or(input.profile_workload_hint)
        .or(input.default_workload_hint)
        .unwrap_or_else(|| {
            if input.has_tcp_target {
                WorkloadHint::Large
            } else {
                WorkloadHint::Concurrent
            }
        })
}

fn implicit_transport_pool_size(input: &RoutePoolSizingInput, hint: WorkloadHint) -> usize {
    match hint {
        WorkloadHint::Large => 1,
        WorkloadHint::Concurrent | WorkloadHint::Mixed => {
            if input.has_tcp_target {
                1
            } else {
                4
            }
        }
    }
}

fn implicit_transport_pool_reason(input: &RoutePoolSizingInput, hint: WorkloadHint) -> String {
    match (input.has_tcp_target, hint) {
        (true, WorkloadHint::Large) => {
            "pool_policy=large: implicit single-worker default for fixed --tcp-target routes"
                .to_string()
        }
        (true, _) => {
            format!(
                "pool_policy={}: fixed --tcp-target routes stay at pool=1 unless --transport-pool-size is explicit",
                pool_policy_name(hint)
            )
        }
        (false, WorkloadHint::Large) => {
            "pool_policy=large: single-worker default favors one large transfer".to_string()
        }
        (false, WorkloadHint::Concurrent) => {
            "pool_policy=concurrent: implicit pool=4 default for multi-flow SOCKS/HTTP proxy routes"
                .to_string()
        }
        (false, WorkloadHint::Mixed) => {
            "pool_policy=mixed: implicit pool=4 default balances large and concurrent proxy traffic"
                .to_string()
        }
    }
}

pub fn pool_policy_name(hint: WorkloadHint) -> &'static str {
    match hint {
        WorkloadHint::Large => "large",
        WorkloadHint::Concurrent => "concurrent",
        WorkloadHint::Mixed => "mixed",
    }
}

fn implicit_ssh_session_pool_size(input: &RoutePoolSizingInput) -> usize {
    if input.has_tcp_target { 1 } else { 2 }
}

fn implicit_ssh_session_pool_reason(input: &RoutePoolSizingInput) -> String {
    if input.has_tcp_target {
        "implicit ssh-native single-session default for fixed --tcp-target routes".to_string()
    } else {
        "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes".to_string()
    }
}

fn ssh_session_pool_warning(size: usize) -> Option<String> {
    (size > 2).then(|| {
        "ssh-native session pools above 2 can lose to handshake and scheduling overhead; benchmark before relying on this explicit value"
            .to_string()
    })
}

fn pool_reason(source: &str, requested: usize, effective: usize) -> String {
    if requested == effective {
        format!("loaded from {source}")
    } else {
        format!("loaded from {source}; clamped to minimum 1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_pool_uses_implicit_concurrent_default() {
        let policy = plan_transport_pool(&RoutePoolSizingInput::default());

        assert_eq!(policy.size, 4);
        assert_eq!(policy.source, "implicit");
        assert_eq!(policy.pool_policy, "concurrent");
        assert_eq!(policy.workload_hint, WorkloadHint::Concurrent);
    }

    #[test]
    fn fixed_tcp_target_stays_single_worker() {
        let policy = plan_transport_pool(&RoutePoolSizingInput {
            has_tcp_target: true,
            command_workload_hint: Some(WorkloadHint::Mixed),
            ..Default::default()
        });

        assert_eq!(policy.size, 1);
        assert_eq!(policy.pool_policy, "mixed");
        assert!(policy.reason.contains("fixed --tcp-target"));
    }

    #[test]
    fn explicit_pool_sizes_are_clamped_and_reported() {
        let policy = plan_transport_pool(&RoutePoolSizingInput {
            command_transport_pool_size: Some(0),
            ..Default::default()
        });

        assert_eq!(policy.size, 1);
        assert_eq!(policy.source, "command-line");
        assert!(policy.reason.contains("clamped to minimum 1"));
    }

    #[test]
    fn default_ssh_pool_is_capped_for_safe_implicit_use() {
        let policy = plan_ssh_session_pool(&RoutePoolSizingInput {
            default_ssh_session_pool_size: Some(8),
            ..Default::default()
        });

        assert_eq!(policy.size, 2);
        assert_eq!(policy.source, "defaults");
        assert!(policy.reason.contains("capped to pool=2"));
        assert!(policy.warning.is_some());
    }
}
