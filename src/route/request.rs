use serde_json::json;

use crate::{cli, node_daemon};

pub(crate) fn route_intent_request(args: cli::RouteArgs) -> serde_json::Value {
    node_daemon::NodeRequest::route_intent(args)
        .to_value()
        .expect("route intent request should serialize")
}

pub(crate) fn route_start_request(
    id: &str,
    forward: cli::NodeForwardArgs,
    persist: bool,
) -> serde_json::Value {
    let proxy = node_daemon::proxy_args_from_node_forward(forward);
    node_daemon::NodeRequest::route_start_forward(id, persist, proxy)
        .to_value()
        .expect("forward route request should serialize")
}

pub(crate) fn route_start_request_with_reason(
    id: &str,
    forward: cli::NodeForwardArgs,
    persist: bool,
    fallback_reason: Option<String>,
) -> serde_json::Value {
    let proxy = node_daemon::proxy_args_from_node_forward(forward);
    let mut request = node_daemon::NodeRequest::route_start_forward(id, persist, proxy)
        .to_value()
        .expect("forward route request should serialize");
    if let Some(reason) = fallback_reason {
        if let Some(object) = request.as_object_mut() {
            object.insert("fallback_reason".to_string(), json!(reason));
        }
    }
    request
}

pub(crate) fn reverse_route_start_request(
    id: &str,
    reverse: cli::NodeReverseArgs,
    persist: bool,
) -> serde_json::Value {
    let reverse = node_daemon::reverse_args_from_node_reverse(reverse);
    node_daemon::NodeRequest::route_start_reverse(
        id,
        persist,
        reverse,
        Some("reverse-link".to_string()),
    )
    .to_value()
    .expect("reverse route request should serialize")
}
