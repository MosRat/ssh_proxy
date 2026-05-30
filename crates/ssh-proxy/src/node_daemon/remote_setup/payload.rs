use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};
use ssh_proxy_deploy::{
    RemoteSetupPayloadInput, build_proxy_env as deploy_build_proxy_env, build_remote_setup_payload,
};

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
    build_remote_setup_payload(RemoteSetupPayloadInput {
        target: spec.target.clone(),
        workspace_id: spec.key().to_string(),
        workspace_paths: spec.workspace_paths.clone(),
        remote_url: remote_url.to_string(),
        bind_host: spec.remote_bind.to_string(),
        port: remote_port_from_url(remote_url).unwrap_or(spec.remote_port_policy.preferred),
        connect_mode: spec.connect_mode.to_string(),
        route_id: spec.route_id(),
        job_id: spec.job_id(),
        route_owner: route
            .and_then(|route| route.get("owner"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        selected_transport: route
            .and_then(|route| route.get("selected_transport"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        fallback_reason: route
            .and_then(|route| route.get("fallback_reason"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        local_proxy: spec.local_proxy.clone(),
        server_dir: spec.apply_policy.server_dir.clone(),
        no_proxy: spec.apply_policy.no_proxy.clone(),
        proxy_support: spec.apply_policy.proxy_support.clone(),
        terminal_env: spec.apply_policy.terminal_env,
    })
}

pub(super) fn build_proxy_env(proxy_url: &str, no_proxy: &str) -> BTreeMap<String, String> {
    deploy_build_proxy_env(proxy_url, no_proxy)
}

fn remote_port_from_url(url: &str) -> Option<u16> {
    let (_, rest) = url.split_once("://")?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..authority_end]
        .rsplit_once('@')
        .map(|(_, endpoint)| endpoint)
        .unwrap_or(&rest[..authority_end]);
    if let Some(stripped) = authority.strip_prefix('[') {
        let (_, tail) = stripped.split_once(']')?;
        return tail.strip_prefix(':')?.parse().ok();
    }
    authority.rsplit_once(':')?.1.parse().ok()
}
