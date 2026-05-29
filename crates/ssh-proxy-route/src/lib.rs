use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::report::RuntimeDecisionReport;

mod policy;
mod status;

pub use policy::{
    RoutePoolSizingInput, RouteSshSessionPoolPolicy, RouteTransportPoolPolicy,
    plan_ssh_session_pool, plan_transport_pool, pool_policy_name,
};
pub use status::{RouteReadinessReport, RouteStats, RouteStatusReport, RouteTaskRecord};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePreflightReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_fallback: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_failures: Vec<Value>,
}

impl RoutePreflightReport {
    pub fn is_empty(&self) -> bool {
        self.recommended_fallback.is_none()
            && self.selected_reason.is_none()
            && self.repair_hint.is_none()
            && self.candidate_failures.is_empty()
    }

    pub fn to_json(&self) -> Value {
        if self.is_empty() {
            return Value::Null;
        }
        json!({
            "recommended_fallback": self.recommended_fallback.as_deref(),
            "selected_reason": self.selected_reason.as_deref().unwrap_or("unknown"),
            "repair_hint": self.repair_hint.as_deref().unwrap_or("unknown"),
            "candidate_failures": self.candidate_failures,
        })
    }

    fn has_recommended_fallback(&self) -> bool {
        self.recommended_fallback.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSessionPoolReport {
    pub size: usize,
    pub source: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportPoolReport {
    pub size: usize,
    pub source: String,
    pub reason: String,
    pub pool_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteRuntimeContext {
    pub selected_transport: String,
    pub transport_selection_source: String,
    pub transport_selection_reason: String,
    pub direct_transport_policy: Value,
    pub direct_transport_policy_reason: Value,
    pub tls_peer_auth_mode: Value,
    pub ssh_mode: Value,
    pub ssh_mode_reason: Value,
    pub ssh_data_plane_reason: Value,
    pub requires_external_ssh: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preflight: Option<RoutePreflightReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssh_session_pool: Option<SshSessionPoolReport>,
    pub transport_pool: TransportPoolReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workload_hint: Option<String>,
    pub connect_timeout_secs: u64,
    pub reconnect_delay_secs: u64,
    pub reconnect_max_delay_secs: u64,
    pub no_reconnect: bool,
}

impl RouteRuntimeContext {
    pub fn decision_report(&self) -> RuntimeDecisionReport {
        let mut report = RuntimeDecisionReport::new(
            &self.selected_transport,
            &self.transport_selection_source,
            &self.transport_selection_reason,
        )
        .requires_external_ssh(self.requires_external_ssh)
        .with_details(json!({
            "direct_transport_policy": self.direct_transport_policy.clone(),
            "direct_transport_policy_reason": self.direct_transport_policy_reason.clone(),
            "tls_peer_auth_mode": self.tls_peer_auth_mode.clone(),
            "ssh_mode": self.ssh_mode.clone(),
            "ssh_mode_reason": self.ssh_mode_reason.clone(),
            "ssh_data_plane_reason": self.ssh_data_plane_reason.clone(),
            "transport_pool_size": self.transport_pool.size,
            "transport_pool_source": self.transport_pool.source,
            "transport_pool_reason": self.transport_pool.reason,
            "pool_policy": self.transport_pool.pool_policy,
            "workload_hint": self.workload_hint,
        }));
        if let Some(endpoint) = &self.selected_endpoint {
            report = report.with_endpoint(endpoint);
        }
        report
    }

    pub fn to_metadata_value(&self) -> Value {
        let connection_decision =
            serde_json::to_value(self.decision_report()).unwrap_or(Value::Null);
        let preflight = self.preflight_json();
        json!({
            "selected_transport": self.selected_transport,
            "transport_selection_source": self.transport_selection_source,
            "transport_selection_reason": self.transport_selection_reason,
            "connection_decision": connection_decision,
            "direct_transport_policy": self.direct_transport_policy.clone(),
            "direct_transport_policy_reason": self.direct_transport_policy_reason.clone(),
            "tls_peer_auth_mode": self.tls_peer_auth_mode.clone(),
            "preflight": preflight,
            "decision_chain": self.decision_chain_value(preflight),
            "ssh_mode": self.ssh_mode.clone(),
            "ssh_mode_reason": self.ssh_mode_reason.clone(),
            "ssh_data_plane_reason": self.ssh_data_plane_reason.clone(),
            "ssh_session_pool_size": self.ssh_session_pool.as_ref().map(|pool| pool.size),
            "ssh_session_pool_source": self.ssh_session_pool.as_ref().map(|pool| pool.source.as_str()),
            "ssh_session_pool_reason": self.ssh_session_pool.as_ref().map(|pool| pool.reason.as_str()),
            "ssh_session_pool_warning": self.ssh_session_pool.as_ref().and_then(|pool| pool.warning.as_deref()),
            "transport_pool_size": self.transport_pool.size,
            "transport_pool_source": self.transport_pool.source,
            "transport_pool_reason": self.transport_pool.reason,
            "pool_policy": self.transport_pool.pool_policy,
            "workload_hint": self.workload_hint,
            "connect_timeout_secs": self.connect_timeout_secs,
            "reconnect_delay_secs": self.reconnect_delay_secs,
            "reconnect_max_delay_secs": self.reconnect_max_delay_secs,
            "no_reconnect": self.no_reconnect,
        })
    }

    fn preflight_json(&self) -> Value {
        self.preflight
            .as_ref()
            .map(RoutePreflightReport::to_json)
            .unwrap_or(Value::Null)
    }

    fn decision_chain_value(&self, preflight: Value) -> Value {
        let has_recommended_fallback = self
            .preflight
            .as_ref()
            .is_some_and(RoutePreflightReport::has_recommended_fallback);
        let topology_class = if has_recommended_fallback {
            "ssh-only"
        } else {
            "runtime-materialized"
        };
        json!({
            "preflight": preflight,
            "topology": {
                "class": topology_class,
            },
            "policy": {
                "direct_transport_policy": self.direct_transport_policy.clone(),
                "direct_transport_policy_reason": self.direct_transport_policy_reason.clone(),
                "tls_peer_auth_mode": self.tls_peer_auth_mode.clone(),
                "ssh_data_plane_reason": self.ssh_data_plane_reason.clone(),
                "explicit_user_override": matches!(
                    self.transport_selection_source.as_str(),
                    "cli" | "profile"
                ),
                "selection_source": self.transport_selection_source,
            },
            "workload": {
                "hint": self.workload_hint,
                "pool_policy": self.transport_pool.pool_policy,
                "transport_pool_size": self.transport_pool.size,
            },
            "selected_transport": self.selected_transport,
            "selected_reason": self.transport_selection_reason,
            "fallback_reason": if has_recommended_fallback {
                Some(self.transport_selection_reason.as_str())
            } else {
                None
            },
            "next_action": if has_recommended_fallback {
                "using materialized preflight selection"
            } else {
                "none"
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RouteRuntimeContext {
        RouteRuntimeContext {
            selected_transport: "ssh-exec".to_string(),
            transport_selection_source: "cli".to_string(),
            transport_selection_reason: "explicit compatibility".to_string(),
            direct_transport_policy: Value::Null,
            direct_transport_policy_reason: Value::Null,
            tls_peer_auth_mode: Value::Null,
            ssh_mode: json!("external"),
            ssh_mode_reason: json!("explicit"),
            ssh_data_plane_reason: json!("compatibility"),
            requires_external_ssh: true,
            selected_endpoint: None,
            preflight: Some(RoutePreflightReport {
                recommended_fallback: Some("ssh-exec".to_string()),
                selected_reason: Some("plain tcp unavailable".to_string()),
                repair_hint: Some("install remote peer".to_string()),
                candidate_failures: vec![json!({"transport": "tls", "error": "failed"})],
            }),
            ssh_session_pool: Some(SshSessionPoolReport {
                size: 2,
                source: "implicit".to_string(),
                reason: "default".to_string(),
                warning: None,
            }),
            transport_pool: TransportPoolReport {
                size: 4,
                source: "implicit".to_string(),
                reason: "concurrent default".to_string(),
                pool_policy: "concurrent".to_string(),
            },
            workload_hint: Some("concurrent".to_string()),
            connect_timeout_secs: 10,
            reconnect_delay_secs: 1,
            reconnect_max_delay_secs: 30,
            no_reconnect: false,
        }
    }

    #[test]
    fn metadata_preserves_legacy_runtime_shape() {
        let value = context().to_metadata_value();

        assert_eq!(value["selected_transport"], "ssh-exec");
        assert_eq!(
            value["connection_decision"]["selected_transport"],
            "ssh-exec"
        );
        assert_eq!(value["connection_decision"]["requires_external_ssh"], true);
        assert_eq!(value["decision_chain"]["topology"]["class"], "ssh-only");
        assert_eq!(value["ssh_session_pool_size"], 2);
        assert_eq!(value["transport_pool_size"], 4);
    }

    #[test]
    fn empty_preflight_renders_null() {
        let report = RoutePreflightReport::default();

        assert_eq!(report.to_json(), Value::Null);
    }
}
