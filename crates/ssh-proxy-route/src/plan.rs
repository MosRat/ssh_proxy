use serde_json::{Map, Value, json};

use crate::SshSessionPoolReport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRuntimePlanReport {
    pub reconnect_delay_secs: u64,
    pub reconnect_max_delay_secs: u64,
    pub connect_timeout_secs: u64,
    pub transport_pool_size: usize,
    pub transport_pool_source: String,
    pub transport_pool_reason: String,
    pub pool_policy: String,
    pub workload_hint: String,
    pub no_reconnect: bool,
}

impl RouteRuntimePlanReport {
    pub fn to_json(&self) -> Value {
        json!({
            "reconnect_delay_secs": self.reconnect_delay_secs,
            "reconnect_max_delay_secs": self.reconnect_max_delay_secs,
            "connect_timeout_secs": self.connect_timeout_secs,
            "transport_pool_size": self.transport_pool_size,
            "transport_pool_source": self.transport_pool_source,
            "transport_pool_reason": self.transport_pool_reason,
            "pool_policy": self.pool_policy,
            "workload_hint": self.workload_hint,
            "no_reconnect": self.no_reconnect,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoutePlanReport {
    pub route_id: String,
    pub direction: String,
    pub owner: String,
    pub mode: String,
    pub listener: Value,
    pub egress: Value,
    pub transport_candidates: Vec<String>,
    pub selected_transport: String,
    pub transport_selection_source: Option<String>,
    pub transport_selection_reason: Option<String>,
    pub direct_transport_policy: Value,
    pub direct_transport_policy_reason: Value,
    pub tls_peer_auth_mode: Value,
    pub ssh_mode: Value,
    pub ssh_mode_reason: Value,
    pub ssh_data_plane_reason: Value,
    pub include_ssh_session_pool_fields: bool,
    pub ssh_session_pool: Option<SshSessionPoolReport>,
    pub topology: Option<Value>,
    pub preflight: Option<Value>,
    pub runtime: RouteRuntimePlanReport,
    pub fallback_reason: Option<String>,
    pub next_action: String,
    pub persist: bool,
}

impl RoutePlanReport {
    pub fn to_json(&self) -> Value {
        let runtime = self.runtime.to_json();
        let mut object = Map::new();
        object.insert("route_id".to_string(), json!(self.route_id));
        object.insert("direction".to_string(), json!(self.direction));
        object.insert("owner".to_string(), json!(self.owner));
        object.insert("mode".to_string(), json!(self.mode));
        object.insert("listener".to_string(), self.listener.clone());
        object.insert("egress".to_string(), self.egress.clone());
        object.insert(
            "transport_candidates".to_string(),
            json!(self.transport_candidates),
        );
        object.insert(
            "selected_transport".to_string(),
            json!(self.selected_transport),
        );
        if let Some(source) = &self.transport_selection_source {
            object.insert("transport_selection_source".to_string(), json!(source));
        }
        if let Some(reason) = &self.transport_selection_reason {
            object.insert("transport_selection_reason".to_string(), json!(reason));
        }
        if self.has_transport_metadata() {
            object.insert(
                "direct_transport_policy".to_string(),
                self.direct_transport_policy.clone(),
            );
            object.insert(
                "direct_transport_policy_reason".to_string(),
                self.direct_transport_policy_reason.clone(),
            );
            object.insert(
                "tls_peer_auth_mode".to_string(),
                self.tls_peer_auth_mode.clone(),
            );
            object.insert("ssh_mode".to_string(), self.ssh_mode.clone());
            object.insert("ssh_mode_reason".to_string(), self.ssh_mode_reason.clone());
            object.insert(
                "ssh_data_plane_reason".to_string(),
                self.ssh_data_plane_reason.clone(),
            );
        }
        if self.include_ssh_session_pool_fields {
            insert_ssh_session_pool(&mut object, self.ssh_session_pool.as_ref());
        }
        if let Some(topology) = &self.topology {
            object.insert("topology".to_string(), topology.clone());
        }
        if let Some(preflight) = &self.preflight {
            object.insert("preflight".to_string(), preflight.clone());
        }
        object.insert("runtime".to_string(), runtime.clone());
        object.insert("fallback_reason".to_string(), json!(self.fallback_reason));
        object.insert("next_action".to_string(), json!(self.next_action));
        object.insert("persist".to_string(), json!(self.persist));
        object.insert(
            "decision_chain".to_string(),
            self.decision_chain_value(&runtime),
        );
        Value::Object(object)
    }

    fn has_transport_metadata(&self) -> bool {
        self.transport_selection_source.is_some()
            || self.transport_selection_reason.is_some()
            || !self.direct_transport_policy.is_null()
            || !self.direct_transport_policy_reason.is_null()
            || !self.tls_peer_auth_mode.is_null()
            || !self.ssh_mode.is_null()
            || !self.ssh_mode_reason.is_null()
            || !self.ssh_data_plane_reason.is_null()
    }

    fn decision_chain_value(&self, runtime: &Value) -> Value {
        let preflight = self.preflight.as_ref();
        let topology = self.topology.as_ref();
        let selection_source = self
            .transport_selection_source
            .as_deref()
            .unwrap_or("unknown");
        let selection_reason = self
            .transport_selection_reason
            .as_deref()
            .unwrap_or("unknown");
        json!({
            "preflight": {
                "kind": preflight.and_then(|value| value.get("kind")).cloned().unwrap_or(Value::Null),
                "recommended_fallback": preflight.and_then(|value| value.get("recommended_fallback")).cloned().unwrap_or(Value::Null),
                "selected_reason": preflight.and_then(|value| value.get("selected_reason")).cloned().unwrap_or(Value::Null),
                "repair_hint": preflight.and_then(|value| value.get("repair_hint")).cloned().unwrap_or(Value::Null),
                "candidate_failures": preflight.and_then(|value| value.get("candidate_failures")).cloned().unwrap_or_else(|| json!([])),
            },
            "topology": {
                "class": route_topology_class(preflight, topology),
                "ssh_jump_chain": topology.and_then(|value| value.get("ssh_jump_chain")).cloned().unwrap_or_else(|| json!([])),
                "direct_private_candidates": topology.and_then(|value| value.get("direct_private_candidates")).cloned().unwrap_or_else(|| json!([])),
            },
            "policy": {
                "direct_transport_policy": self.direct_transport_policy.clone(),
                "direct_transport_policy_reason": self.direct_transport_policy_reason.clone(),
                "tls_peer_auth_mode": self.tls_peer_auth_mode.clone(),
                "ssh_data_plane_reason": self.ssh_data_plane_reason.clone(),
                "explicit_user_override": matches!(selection_source, "cli" | "profile"),
                "selection_source": selection_source,
            },
            "workload": {
                "hint": runtime.get("workload_hint").cloned().unwrap_or(Value::Null),
                "pool_policy": runtime.get("pool_policy").cloned().unwrap_or(Value::Null),
                "transport_pool_size": runtime.get("transport_pool_size").cloned().unwrap_or(Value::Null),
            },
            "selected_transport": self.selected_transport,
            "selected_reason": selection_reason,
            "fallback_reason": self.fallback_reason,
            "next_action": self.next_action,
        })
    }
}

pub fn refresh_route_decision_chain(plan: &mut Value) {
    let Some(report) = RoutePlanReport::from_legacy_plan(plan) else {
        return;
    };
    if let Some(object) = plan.as_object_mut() {
        object.insert(
            "decision_chain".to_string(),
            report.decision_chain_value(&report.runtime.to_json()),
        );
    }
}

fn insert_ssh_session_pool(object: &mut Map<String, Value>, pool: Option<&SshSessionPoolReport>) {
    object.insert(
        "ssh_session_pool_size".to_string(),
        pool.map(|pool| json!(pool.size)).unwrap_or(Value::Null),
    );
    object.insert(
        "ssh_session_pool_source".to_string(),
        pool.map(|pool| json!(pool.source.as_str()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "ssh_session_pool_reason".to_string(),
        pool.map(|pool| json!(pool.reason.as_str()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "ssh_session_pool_warning".to_string(),
        pool.and_then(|pool| pool.warning.as_deref())
            .map(|warning| json!(warning))
            .unwrap_or(Value::Null),
    );
}

impl RoutePlanReport {
    fn from_legacy_plan(plan: &Value) -> Option<Self> {
        let runtime = plan.get("runtime").unwrap_or(&Value::Null);
        Some(Self {
            route_id: plan.get("route_id")?.as_str()?.to_string(),
            direction: plan.get("direction")?.as_str()?.to_string(),
            owner: plan.get("owner")?.as_str()?.to_string(),
            mode: plan.get("mode")?.as_str()?.to_string(),
            listener: plan.get("listener").cloned().unwrap_or(Value::Null),
            egress: plan.get("egress").cloned().unwrap_or(Value::Null),
            transport_candidates: plan
                .get("transport_candidates")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            selected_transport: plan
                .get("selected_transport")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            transport_selection_source: plan
                .get("transport_selection_source")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            transport_selection_reason: plan
                .get("transport_selection_reason")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            direct_transport_policy: plan
                .get("direct_transport_policy")
                .cloned()
                .unwrap_or(Value::Null),
            direct_transport_policy_reason: plan
                .get("direct_transport_policy_reason")
                .cloned()
                .unwrap_or(Value::Null),
            tls_peer_auth_mode: plan
                .get("tls_peer_auth_mode")
                .cloned()
                .unwrap_or(Value::Null),
            ssh_mode: plan.get("ssh_mode").cloned().unwrap_or(Value::Null),
            ssh_mode_reason: plan.get("ssh_mode_reason").cloned().unwrap_or(Value::Null),
            ssh_data_plane_reason: plan
                .get("ssh_data_plane_reason")
                .cloned()
                .unwrap_or(Value::Null),
            include_ssh_session_pool_fields: plan.get("ssh_session_pool_size").is_some(),
            ssh_session_pool: None,
            topology: plan.get("topology").cloned(),
            preflight: plan.get("preflight").cloned(),
            runtime: RouteRuntimePlanReport {
                reconnect_delay_secs: runtime_u64(runtime, "reconnect_delay_secs"),
                reconnect_max_delay_secs: runtime_u64(runtime, "reconnect_max_delay_secs"),
                connect_timeout_secs: runtime_u64(runtime, "connect_timeout_secs"),
                transport_pool_size: runtime_usize(runtime, "transport_pool_size"),
                transport_pool_source: runtime_string(runtime, "transport_pool_source", "implicit"),
                transport_pool_reason: runtime_string(
                    runtime,
                    "transport_pool_reason",
                    "implicit single-worker default",
                ),
                pool_policy: runtime_string(runtime, "pool_policy", "large"),
                workload_hint: runtime_string(runtime, "workload_hint", "large"),
                no_reconnect: runtime
                    .get("no_reconnect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            fallback_reason: plan
                .get("fallback_reason")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            next_action: plan
                .get("next_action")
                .and_then(Value::as_str)
                .unwrap_or("none")
                .to_string(),
            persist: plan
                .get("persist")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

fn runtime_u64(runtime: &Value, key: &str) -> u64 {
    runtime.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn runtime_usize(runtime: &Value, key: &str) -> usize {
    runtime
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn runtime_string(runtime: &Value, key: &str, default: &str) -> String {
    runtime
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn route_topology_class(preflight: Option<&Value>, topology: Option<&Value>) -> &'static str {
    let direct_reachable = preflight
        .and_then(|value| value.get("results"))
        .and_then(Value::as_array)
        .map(|results| {
            results.iter().any(|result| {
                is_direct_probe_protocol(result.get("protocol").and_then(Value::as_str))
                    && result.get("reachable") == Some(&Value::Bool(true))
            })
        })
        .unwrap_or(false);
    if direct_reachable {
        return "direct-reachable";
    }

    let recommended_fallback = preflight
        .and_then(|value| value.get("recommended_fallback"))
        .and_then(Value::as_str);
    if recommended_fallback.is_some() {
        return "ssh-only";
    }

    let has_jump = topology
        .and_then(|value| value.get("ssh_jump_chain"))
        .and_then(Value::as_array)
        .map(|chain| !chain.is_empty())
        .unwrap_or(false);
    let has_direct_candidates = topology
        .and_then(|value| value.get("direct_private_candidates"))
        .and_then(Value::as_array)
        .map(|candidates| !candidates.is_empty())
        .unwrap_or(false);
    if has_jump && has_direct_candidates {
        return "mixed";
    }
    if has_direct_candidates {
        return "unknown-direct";
    }
    "ssh-reachable"
}

fn is_direct_probe_protocol(protocol: Option<&str>) -> bool {
    matches!(protocol, Some("quic" | "tls-tcp" | "plain-tcp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> RoutePlanReport {
        RoutePlanReport {
            route_id: "route-1".to_string(),
            direction: "local-uses-remote".to_string(),
            owner: "local".to_string(),
            mode: "local-forward".to_string(),
            listener: json!({"owner": "local"}),
            egress: json!({"peer": "node"}),
            transport_candidates: vec!["tls-tcp".to_string(), "ssh-native".to_string()],
            selected_transport: "tls-tcp".to_string(),
            transport_selection_source: Some("topology".to_string()),
            transport_selection_reason: Some("direct endpoint".to_string()),
            direct_transport_policy: json!("production_direct"),
            direct_transport_policy_reason: json!("TLS baseline"),
            tls_peer_auth_mode: json!("server_auth"),
            ssh_mode: Value::Null,
            ssh_mode_reason: Value::Null,
            ssh_data_plane_reason: Value::Null,
            include_ssh_session_pool_fields: true,
            ssh_session_pool: None,
            topology: Some(json!({
                "ssh_jump_chain": [],
                "direct_private_candidates": ["tls-tcp://192.0.2.8:19082"]
            })),
            preflight: None,
            runtime: RouteRuntimePlanReport {
                reconnect_delay_secs: 1,
                reconnect_max_delay_secs: 30,
                connect_timeout_secs: 10,
                transport_pool_size: 4,
                transport_pool_source: "implicit".to_string(),
                transport_pool_reason: "default".to_string(),
                pool_policy: "large".to_string(),
                workload_hint: "large".to_string(),
                no_reconnect: false,
            },
            fallback_reason: None,
            next_action: "none".to_string(),
            persist: true,
        }
    }

    #[test]
    fn route_plan_report_preserves_legacy_fields() {
        let value = report().to_json();

        assert_eq!(value["route_id"], "route-1");
        assert_eq!(value["selected_transport"], "tls-tcp");
        assert_eq!(value["ssh_session_pool_size"], Value::Null);
        assert_eq!(
            value["decision_chain"]["policy"]["direct_transport_policy"],
            "production_direct"
        );
        assert_eq!(
            value["decision_chain"]["topology"]["class"],
            "unknown-direct"
        );
    }

    #[test]
    fn refresh_route_decision_chain_preserves_preflight_updates() {
        let mut value = report().to_json();
        value["preflight"] = json!({
            "kind": "local-direct-transport-probe",
            "recommended_fallback": "ssh-native",
            "selected_reason": "direct failed",
            "repair_hint": "publish endpoint",
            "candidate_failures": [{"protocol": "tls-tcp"}],
        });

        refresh_route_decision_chain(&mut value);

        assert_eq!(value["decision_chain"]["topology"]["class"], "ssh-only");
        assert_eq!(
            value["decision_chain"]["preflight"]["recommended_fallback"],
            "ssh-native"
        );
    }
}
