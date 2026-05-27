use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::{cli, node_daemon::response_line, route};

pub(super) fn route_response_with_plan(response: &str, plan: Value) -> Result<String> {
    let mut value: Value = serde_json::from_str(response.trim())
        .context("failed to parse route response before attaching plan")?;
    if let Value::Object(object) = &mut value {
        let route_id = plan
            .get("route_id")
            .cloned()
            .or_else(|| object.get("id").cloned())
            .unwrap_or(Value::Null);
        let selected_transport = plan
            .get("selected_transport")
            .cloned()
            .unwrap_or(Value::Null);
        let connect_mode = plan.get("mode").cloned().unwrap_or(Value::Null);
        let remote_listen = plan
            .pointer("/listener/listen")
            .cloned()
            .unwrap_or_else(|| object.get("listen").cloned().unwrap_or(Value::Null));
        let fallback_reason = plan.get("fallback_reason").cloned().unwrap_or_else(|| {
            object
                .get("fallback_reason")
                .cloned()
                .unwrap_or(Value::Null)
        });
        object.insert("route_id".to_string(), route_id.clone());
        object.insert("selected_transport".to_string(), selected_transport);
        object.insert("connect_mode".to_string(), connect_mode);
        object.insert("remote_listen".to_string(), remote_listen.clone());
        object
            .entry("listen")
            .or_insert_with(|| remote_listen.clone());
        if !object.contains_key("owner") {
            object.insert(
                "owner".to_string(),
                plan.get("owner")
                    .cloned()
                    .unwrap_or_else(|| Value::from("local")),
            );
        }
        object.insert(
            "remote_url".to_string(),
            remote_proxy_url_from_plan(&plan, &remote_listen).unwrap_or(Value::Null),
        );
        object.insert("fallback_reason".to_string(), fallback_reason);
        object.insert(
            "cleanup_command".to_string(),
            route_id
                .as_str()
                .map(|id| format!("ssh_proxy node control stop-route {id}"))
                .map(Value::from)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "health".to_string(),
            json!({
                "state": "accepted",
                "message": "route accepted; query `ssh_proxy node control routes` for live health"
            }),
        );
        object.insert("plan".to_string(), plan);
    }
    response_line(value)
}

pub(super) fn remote_proxy_url_from_plan(plan: &Value, remote_listen: &Value) -> Option<Value> {
    let upstream = plan.pointer("/egress/upstream_proxy")?.as_str()?;
    let listen = remote_listen.as_str()?;
    let (scheme, _) = upstream.split_once("://")?;
    let rest = upstream.split_once("://")?.1;
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = &rest[authority_end..];
    let userinfo = authority
        .rsplit_once('@')
        .map(|(userinfo, _)| format!("{userinfo}@"))
        .unwrap_or_default();
    Some(Value::from(format!(
        "{scheme}://{userinfo}{listen}{suffix}"
    )))
}

pub(super) fn remote_direct_route_response(target: &str, plan: Value) -> Value {
    let route_id = plan.get("route_id").cloned().unwrap_or(Value::Null);
    let remote_listen = plan
        .pointer("/listener/listen")
        .cloned()
        .unwrap_or(Value::Null);
    let cleanup_command = route_id
        .as_str()
        .map(|id| format!("ssh_proxy host {target} node-stop-route {id}"))
        .map(Value::from)
        .unwrap_or(Value::Null);
    json!({
        "ok": true,
        "message": "remote route intent accepted",
        "route_id": route_id,
        "owner": "remote",
        "mode": "direct",
        "connect_mode": "direct",
        "selected_transport": plan.get("selected_transport").cloned().unwrap_or(Value::Null),
        "listen": remote_listen.clone(),
        "remote_listen": remote_listen.clone(),
        "remote_url": remote_proxy_url_from_plan(&plan, &remote_listen).unwrap_or(Value::Null),
        "fallback_reason": plan.get("fallback_reason").cloned().unwrap_or(Value::Null),
        "cleanup_command": cleanup_command,
        "health": {
            "state": "accepted",
            "message": "remote-owned route accepted; query the remote node for live health"
        },
        "plan": plan
    })
}

pub(super) fn remote_direct_route_plan(
    args: &cli::RouteArgs,
    command: &cli::HostCommand,
    local_peer: std::net::SocketAddr,
) -> Value {
    match command {
        cli::HostCommand::NodeForward(forward) => route::remote_uses_local_direct_plan(
            args,
            forward.id.as_deref().unwrap_or("remote-via-local"),
            forward,
            local_peer,
        ),
        _ => Value::Null,
    }
}
