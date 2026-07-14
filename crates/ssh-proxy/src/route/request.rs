use serde_json::json;

use crate::{cli, node_daemon};

pub(crate) fn route_intent_request(args: cli::RouteArgs) -> serde_json::Value {
    request_value(node_daemon::NodeRequest::route_intent(args), "route_intent")
}

pub(crate) fn route_start_request(
    id: &str,
    forward: cli::NodeForwardArgs,
    persist: bool,
) -> serde_json::Value {
    let proxy = node_daemon::proxy_args_from_node_forward(forward);
    request_value(
        node_daemon::NodeRequest::route_start_forward(id, persist, proxy),
        "route_start",
    )
}

pub(crate) fn route_start_request_with_reason(
    id: &str,
    forward: cli::NodeForwardArgs,
    persist: bool,
    fallback_reason: Option<String>,
) -> serde_json::Value {
    let proxy = node_daemon::proxy_args_from_node_forward(forward);
    let mut request = request_value(
        node_daemon::NodeRequest::route_start_forward(id, persist, proxy),
        "route_start",
    );
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
    request_value(
        node_daemon::NodeRequest::route_start_reverse(
            id,
            persist,
            reverse,
            Some("reverse-link".to_string()),
        ),
        "route_start",
    )
}

fn request_value(request: node_daemon::NodeRequest, cmd: &str) -> serde_json::Value {
    request
        .to_value()
        .unwrap_or_else(|err| request_encode_error(cmd, err))
}

fn request_encode_error(cmd: &str, err: anyhow::Error) -> serde_json::Value {
    json!({
        "api_version": node_daemon::control_api_version(),
        "cmd": cmd,
        "encode_error": err.to_string(),
    })
}
