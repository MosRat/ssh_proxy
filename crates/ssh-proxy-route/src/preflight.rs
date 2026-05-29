use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::model::TransportMode;

use crate::{
    SshSessionPoolReport, refresh_route_decision_chain, remote_transport_name,
    ssh_data_plane_reason, ssh_mode_name, ssh_mode_reason,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteProbeResult {
    pub protocol: String,
    pub endpoint: Option<String>,
    pub reachable: Option<bool>,
    pub status: String,
    pub message: String,
}

impl RouteProbeResult {
    pub fn new(
        protocol: impl Into<String>,
        endpoint: Option<String>,
        reachable: Option<bool>,
        status: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            protocol: protocol.into(),
            endpoint,
            reachable,
            status: status.into(),
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn is_direct_candidate(&self) -> bool {
        is_direct_probe_protocol(&self.protocol)
    }

    fn direct_failed(&self) -> bool {
        self.is_direct_candidate() && self.reachable == Some(false)
    }

    fn direct_reachable(&self) -> bool {
        self.is_direct_candidate() && self.reachable == Some(true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreflightInput {
    pub timeout_ms: u64,
    pub results: Vec<RouteProbeResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreflightDecision {
    pub timeout_ms: u64,
    pub results: Vec<RouteProbeResult>,
    pub candidate_failures: Vec<Value>,
    pub recommended_fallback: Option<String>,
    pub selected_reason: String,
    pub repair_hint: String,
}

impl RoutePreflightDecision {
    pub fn to_plan_value(&self) -> Value {
        json!({
            "kind": "local-direct-transport-probe",
            "timeout_ms": self.timeout_ms,
            "results": self.results_json(),
            "candidate_failures": self.candidate_failures,
            "recommended_fallback": self.recommended_fallback,
            "selected_reason": self.selected_reason,
            "repair_hint": self.repair_hint,
        })
    }

    pub fn results_json(&self) -> Vec<Value> {
        self.results.iter().map(RouteProbeResult::to_json).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteFallbackInput<'a> {
    pub recommended_fallback: Option<&'a str>,
    pub current_transport: TransportMode,
    pub selection_source: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteFallbackDecision {
    pub selected_transport: Option<TransportMode>,
    pub selection_source: Option<String>,
    pub selection_reason: Option<String>,
    pub next_action: Option<String>,
}

impl RouteFallbackDecision {
    pub fn none() -> Self {
        Self {
            selected_transport: None,
            selection_source: None,
            selection_reason: None,
            next_action: None,
        }
    }

    pub fn applied(&self) -> bool {
        self.selected_transport.is_some()
    }

    pub fn reason(&self) -> Option<&str> {
        self.selection_reason.as_deref()
    }

    pub fn apply_to_plan(&self, plan: &mut Value, ssh_session_pool: Option<&SshSessionPoolReport>) {
        let Some(transport) = self.selected_transport else {
            return;
        };
        let selection_source = self
            .selection_source
            .as_deref()
            .unwrap_or("route-preflight");
        let selection_reason = self.selection_reason.as_deref().unwrap_or("unknown");
        let next_action = self.next_action.as_deref().unwrap_or("none");

        if let Some(object) = plan.as_object_mut() {
            object.insert(
                "selected_transport".to_string(),
                json!(remote_transport_name(transport)),
            );
            object.insert(
                "transport_selection_source".to_string(),
                json!(selection_source),
            );
            object.insert(
                "transport_selection_reason".to_string(),
                json!(selection_reason),
            );
            object.insert("ssh_mode".to_string(), ssh_mode_name(transport));
            object.insert("ssh_mode_reason".to_string(), ssh_mode_reason(transport));
            object.insert(
                "ssh_data_plane_reason".to_string(),
                ssh_data_plane_reason(transport, Some(selection_source)),
            );
            if let Some(pool) = ssh_session_pool {
                object.insert("ssh_session_pool_size".to_string(), json!(pool.size));
                object.insert(
                    "ssh_session_pool_source".to_string(),
                    json!(pool.source.as_str()),
                );
                object.insert(
                    "ssh_session_pool_reason".to_string(),
                    json!(pool.reason.as_str()),
                );
                object.insert(
                    "ssh_session_pool_warning".to_string(),
                    json!(pool.warning.as_deref()),
                );
            }
            object.insert("fallback_reason".to_string(), json!(selection_reason));
            object.insert("next_action".to_string(), json!(next_action));
        }
        refresh_route_decision_chain(plan);
    }
}

pub fn decide_route_preflight(input: RoutePreflightInput) -> RoutePreflightDecision {
    let candidate_failures = candidate_failures(&input.results);
    let direct_failures = candidate_failures.len();
    let direct_successes = input
        .results
        .iter()
        .filter(|result| result.direct_reachable())
        .count();
    let recommended_fallback = if direct_failures > 0 && direct_successes == 0 {
        Some("ssh-native".to_string())
    } else {
        None
    };
    let selected_reason = if recommended_fallback.is_some() {
        "all probed direct peer transports failed; SSH fallback is recommended before starting the route"
    } else if direct_successes > 0 {
        "at least one direct peer transport was reachable before route start"
    } else {
        "no failing direct peer transport was observed before route start"
    };
    let repair_hint = if recommended_fallback.is_some() {
        "use ssh-native fallback, or publish a peer endpoint reachable from this client"
    } else if candidate_failures.is_empty() {
        "none"
    } else {
        "publish a reachable peer endpoint, adjust firewall/NAT, or switch to an SSH fallback transport"
    };

    RoutePreflightDecision {
        timeout_ms: input.timeout_ms,
        results: input.results,
        candidate_failures,
        recommended_fallback,
        selected_reason: selected_reason.to_string(),
        repair_hint: repair_hint.to_string(),
    }
}

pub fn decide_route_fallback(input: RouteFallbackInput<'_>) -> RouteFallbackDecision {
    let Some(recommended) = input.recommended_fallback else {
        return RouteFallbackDecision::none();
    };
    let may_override = input.current_transport == TransportMode::Auto
        || matches!(
            input.selection_source,
            Some("topology" | "benchmark-tuned default")
        );
    if !may_override {
        return RouteFallbackDecision::none();
    }
    if recommended != "ssh-native" && recommended != "ssh-direct-tcpip" {
        return RouteFallbackDecision::none();
    }

    RouteFallbackDecision {
        selected_transport: Some(TransportMode::SshNative),
        selection_source: Some("route-preflight".to_string()),
        selection_reason: Some(
            "direct private transport preflight failed; selected SSH native direct-tcpip fallback"
                .to_string(),
        ),
        next_action: Some("using ssh-native fallback; no user action required".to_string()),
    }
}

pub fn candidate_failures(results: &[RouteProbeResult]) -> Vec<Value> {
    results
        .iter()
        .filter(|result| result.direct_failed())
        .map(RouteProbeResult::to_json)
        .collect()
}

pub fn is_direct_probe_protocol(protocol: &str) -> bool {
    matches!(protocol, "quic" | "tls-tcp" | "plain-tcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_tls() -> RouteProbeResult {
        RouteProbeResult::new(
            "tls-tcp",
            Some("192.0.2.1:9".to_string()),
            Some(false),
            "connect-failed",
            "connection refused",
        )
    }

    #[test]
    fn preflight_recommends_ssh_fallback_when_all_direct_candidates_fail() {
        let decision = decide_route_preflight(RoutePreflightInput {
            timeout_ms: 750,
            results: vec![
                failed_tls(),
                RouteProbeResult::new(
                    "ssh-direct-tcpip",
                    Some("127.0.0.1:8080".to_string()),
                    None,
                    "not-probed",
                    "follows the SSH session",
                ),
            ],
        });

        assert_eq!(decision.recommended_fallback.as_deref(), Some("ssh-native"));
        assert_eq!(decision.candidate_failures[0]["protocol"], "tls-tcp");
        assert!(decision.selected_reason.contains("all probed"));
        assert!(decision.repair_hint.contains("ssh-native fallback"));
    }

    #[test]
    fn preflight_keeps_direct_transport_when_any_candidate_reaches() {
        let decision = decide_route_preflight(RoutePreflightInput {
            timeout_ms: 750,
            results: vec![
                failed_tls(),
                RouteProbeResult::new(
                    "plain-tcp",
                    Some("192.0.2.5:19080".to_string()),
                    Some(true),
                    "reachable",
                    "TCP connect succeeded",
                ),
            ],
        });

        assert_eq!(decision.recommended_fallback, None);
        assert!(decision.selected_reason.contains("at least one"));
        assert_eq!(decision.candidate_failures.len(), 1);
    }

    #[test]
    fn fallback_only_overrides_auto_or_topology_selected_transport() {
        let decision = decide_route_fallback(RouteFallbackInput {
            recommended_fallback: Some("ssh-native"),
            current_transport: TransportMode::TlsTcp,
            selection_source: Some("profile"),
        });
        assert!(!decision.applied());

        let decision = decide_route_fallback(RouteFallbackInput {
            recommended_fallback: Some("ssh-native"),
            current_transport: TransportMode::TlsTcp,
            selection_source: Some("topology"),
        });
        assert!(decision.applied());
        assert_eq!(decision.selected_transport, Some(TransportMode::SshNative));
        assert_eq!(
            decision.selection_source.as_deref(),
            Some("route-preflight")
        );
    }

    #[test]
    fn fallback_plan_update_preserves_legacy_runtime_fields() {
        let decision = decide_route_fallback(RouteFallbackInput {
            recommended_fallback: Some("ssh-native"),
            current_transport: TransportMode::TlsTcp,
            selection_source: Some("topology"),
        });
        let pool = SshSessionPoolReport {
            size: 2,
            source: "implicit".to_string(),
            reason: "two-session default".to_string(),
            warning: None,
        };
        let mut plan = json!({
            "route_id": "route-1",
            "direction": "local-uses-remote",
            "owner": "local",
            "mode": "local-forward",
            "listener": {"owner": "local"},
            "egress": {"peer": "node"},
            "transport_candidates": ["tls-tcp", "ssh-native"],
            "preflight": {
                "recommended_fallback": "ssh-native",
                "candidate_failures": [{"protocol": "tls-tcp"}]
            },
            "transport_pool_size": 4,
            "runtime": {}
        });

        decision.apply_to_plan(&mut plan, Some(&pool));

        assert_eq!(plan["selected_transport"], "ssh-native");
        assert_eq!(plan["transport_selection_source"], "route-preflight");
        assert_eq!(plan["ssh_mode"], "native-direct-tcpip");
        assert_eq!(plan["ssh_session_pool_size"], 2);
        assert_eq!(plan["fallback_reason"], decision.reason().unwrap());
        assert_eq!(
            plan["decision_chain"]["policy"]["ssh_data_plane_reason"],
            "simple_egress"
        );
    }
}
