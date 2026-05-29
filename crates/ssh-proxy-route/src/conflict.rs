use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteConflictRoute {
    pub id: String,
    pub direction: String,
    pub listen: Option<String>,
    pub peer: Option<String>,
    pub spec: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteConflictInput {
    pub id: Option<String>,
    pub direction: String,
    pub listen: String,
    pub peer: Option<String>,
    pub spec: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum RouteConflictDecision {
    Available,
    ReuseExisting { route_id: String },
    DifferentSpec { route_id: String },
    ListenerReserved { route_id: String },
}

impl RouteConflictDecision {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

pub fn decide_route_conflict(
    existing: &[RouteConflictRoute],
    candidate: &RouteConflictInput,
) -> RouteConflictDecision {
    if let Some(id) = candidate.id.as_deref() {
        if let Some(route) = existing.iter().find(|route| route.id == id) {
            if route_matches(route, candidate) {
                return RouteConflictDecision::ReuseExisting {
                    route_id: route.id.clone(),
                };
            }
            return RouteConflictDecision::DifferentSpec {
                route_id: route.id.clone(),
            };
        }
    }

    if let Some(route) = existing
        .iter()
        .find(|route| route_reserves_listener(route, candidate))
    {
        return RouteConflictDecision::ListenerReserved {
            route_id: route.id.clone(),
        };
    }

    RouteConflictDecision::Available
}

pub fn route_matches(route: &RouteConflictRoute, candidate: &RouteConflictInput) -> bool {
    route.direction == candidate.direction
        && route.listen.as_deref() == Some(candidate.listen.as_str())
        && match candidate.peer.as_deref() {
            Some(peer) => route.peer.as_deref() == Some(peer),
            None => true,
        }
        && route_specs_match_values(&route.spec, &candidate.spec)
}

pub fn route_reserves_listener(route: &RouteConflictRoute, candidate: &RouteConflictInput) -> bool {
    route.direction == candidate.direction
        && route.listen.as_deref() == Some(candidate.listen.as_str())
        && (candidate.direction == "forward"
            || candidate
                .peer
                .as_deref()
                .is_some_and(|peer| route.peer.as_deref() == Some(peer)))
}

pub fn route_specs_match_values(left: &Value, right: &Value) -> bool {
    normalized_route_spec(left) == normalized_route_spec(right)
}

fn normalized_route_spec(value: &Value) -> Value {
    let mut value = value.clone();
    let is_reverse = value
        .get("direction")
        .and_then(Value::as_str)
        .is_some_and(|direction| direction.eq_ignore_ascii_case("reverse"));
    if is_reverse {
        if let Some(reverse) = value.get_mut("reverse").and_then(Value::as_object_mut) {
            reverse.remove("identity");
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn route(
        id: &str,
        direction: &str,
        listen: &str,
        peer: Option<&str>,
        spec: Value,
    ) -> RouteConflictRoute {
        RouteConflictRoute {
            id: id.to_string(),
            direction: direction.to_string(),
            listen: Some(listen.to_string()),
            peer: peer.map(ToOwned::to_owned),
            spec,
        }
    }

    #[test]
    fn reuses_existing_route_when_spec_matches() {
        let spec = json!({"direction": "forward", "proxy": {"listen": "127.0.0.1:18080"}});
        let existing = vec![route(
            "route-a",
            "forward",
            "127.0.0.1:18080",
            Some("edge"),
            spec.clone(),
        )];
        let candidate = RouteConflictInput {
            id: Some("route-a".to_string()),
            direction: "forward".to_string(),
            listen: "127.0.0.1:18080".to_string(),
            peer: Some("edge".to_string()),
            spec,
        };

        assert_eq!(
            decide_route_conflict(&existing, &candidate),
            RouteConflictDecision::ReuseExisting {
                route_id: "route-a".to_string()
            }
        );
    }

    #[test]
    fn reverse_spec_matching_ignores_identity_enrichment() {
        let left = json!({
            "direction": "reverse",
            "reverse": {
                "target": "edge",
                "remote_listen": "127.0.0.1:17890",
                "identity": []
            }
        });
        let right = json!({
            "direction": "reverse",
            "reverse": {
                "target": "edge",
                "remote_listen": "127.0.0.1:17890",
                "identity": ["id_rsa", "id_ed25519"]
            }
        });

        assert!(route_specs_match_values(&left, &right));
    }

    #[test]
    fn reports_listener_reservation_for_same_forward_listener() {
        let existing = vec![route(
            "route-a",
            "forward",
            "127.0.0.1:18080",
            Some("edge-a"),
            json!({"direction": "forward", "proxy": {"target": "edge-a"}}),
        )];
        let candidate = RouteConflictInput {
            id: Some("route-b".to_string()),
            direction: "forward".to_string(),
            listen: "127.0.0.1:18080".to_string(),
            peer: Some("edge-b".to_string()),
            spec: json!({"direction": "forward", "proxy": {"target": "edge-b"}}),
        };

        assert_eq!(
            decide_route_conflict(&existing, &candidate),
            RouteConflictDecision::ListenerReserved {
                route_id: "route-a".to_string()
            }
        );
    }
}
