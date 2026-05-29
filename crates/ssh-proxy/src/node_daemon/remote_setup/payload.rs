use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::proxy_session::ProxySessionSpec;

pub(super) fn setup_hash(payload: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn setup_payload(
    spec: &ProxySessionSpec,
    remote_url: &str,
    route: Option<&Value>,
) -> Value {
    let env = build_proxy_env(remote_url, &spec.apply_policy.no_proxy);
    let mut values = serde_json::Map::new();
    values.insert("http.proxy".to_string(), json!(remote_url));
    values.insert(
        "http.proxySupport".to_string(),
        json!(&spec.apply_policy.proxy_support),
    );
    if spec.apply_policy.terminal_env {
        values.insert(
            "terminal.integrated.env.linux".to_string(),
            json!(env.clone()),
        );
        values.insert(
            "terminal.integrated.env.osx".to_string(),
            json!(env.clone()),
        );
        values.insert("terminal.integrated.env.windows".to_string(), json!(env));
    }
    json!({
        "target": &spec.target,
        "workspaceId": &spec.workspace_id,
        "workspacePaths": &spec.workspace_paths,
        "proxyUrl": remote_url,
        "bindHost": spec.remote_bind.to_string(),
        "port": spec.remote_port_policy.preferred,
        "connectMode": &spec.connect_mode,
        "routeId": spec.route_id(),
        "jobId": spec.job_id(),
        "routeOwner": route.and_then(|route| route.get("owner")).and_then(Value::as_str),
        "selectedTransport": route.and_then(|route| route.get("selected_transport")).and_then(Value::as_str),
        "fallbackReason": route.and_then(|route| route.get("fallback_reason")).and_then(Value::as_str),
        "localProxySource": "daemon",
        "localProxyUrl": &spec.local_proxy,
        "backend": "ssh_proxy",
        "server_dir": &spec.apply_policy.server_dir,
        "no_proxy": &spec.apply_policy.no_proxy,
        "proxy_support": &spec.apply_policy.proxy_support,
        "values": values,
    })
}

pub(super) fn build_proxy_env(proxy_url: &str, no_proxy: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("HTTP_PROXY".to_string(), proxy_url.to_string());
    env.insert("HTTPS_PROXY".to_string(), proxy_url.to_string());
    env.insert("ALL_PROXY".to_string(), proxy_url.to_string());
    env.insert("NO_PROXY".to_string(), no_proxy.to_string());
    env.insert("http_proxy".to_string(), proxy_url.to_string());
    env.insert("https_proxy".to_string(), proxy_url.to_string());
    env.insert("all_proxy".to_string(), proxy_url.to_string());
    env.insert("no_proxy".to_string(), no_proxy.to_string());
    env
}
