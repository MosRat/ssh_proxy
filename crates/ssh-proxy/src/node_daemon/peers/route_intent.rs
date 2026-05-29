use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use tracing::info;

use crate::{
    cli, deploy,
    node_daemon::{NodeManager, NodeRequest, response_line},
    route,
};

impl NodeManager {
    pub(in crate::node_daemon) async fn handle_route_intent(
        &self,
        request: NodeRequest,
    ) -> Result<String> {
        let args = request
            .route
            .ok_or_else(|| anyhow!("route_intent requires route args"))?;

        match args.direction {
            cli::RouteDirection::LocalUsesRemote => {
                let config = self.config.lock().await.clone();
                let mut forward =
                    route::node_forward_from_route(&args, &config, args.target.clone(), false)?;
                let id = route::route_id(&args, "local-via-remote");
                let mut plan = route::local_uses_remote_plan(&args, &id, &forward);
                route::add_local_transport_probe_results(&mut plan, &mut forward).await;
                let mut fallback_reason =
                    route::apply_local_forward_fallback(&mut forward, &mut plan);
                if !matches!(forward.remote_transport, cli::RemoteTransport::SshNative) {
                    self.ensure_peer_for_route(&args).await?;
                    let config = self.config.lock().await.clone();
                    forward =
                        route::node_forward_from_route(&args, &config, args.target.clone(), false)?;
                    plan = route::local_uses_remote_plan(&args, &id, &forward);
                    route::add_local_transport_probe_results(&mut plan, &mut forward).await;
                    fallback_reason = route::apply_local_forward_fallback(&mut forward, &mut plan);
                }
                let request = match fallback_reason {
                    Some(reason) => route::route_start_request_with_reason(
                        &id,
                        forward,
                        !args.volatile,
                        Some(reason),
                    ),
                    None => route::route_start_request(&id, forward, !args.volatile),
                };
                let request: NodeRequest = serde_json::from_value(request)
                    .context("failed to build local route request")?;
                let response = self.start_route(request).await?;
                route_response_with_plan(&response, plan)
            }
            cli::RouteDirection::RemoteUsesLocal => {
                let config = self.config.lock().await.clone();
                let decision = route::remote_use_decision(&args, &config)?;
                match decision.plan {
                    route::RemoteUsePlan::ReverseLink => {
                        let reverse = route::node_reverse_from_route(&args, &config)?;
                        let id = route::route_id(&args, "remote-via-local-reverse-link");
                        let plan = route::remote_uses_local_reverse_link_plan(
                            &args,
                            &id,
                            &reverse,
                            decision.fallback_reason.as_deref(),
                        );
                        let request =
                            route::reverse_route_start_request(&id, reverse, !args.volatile);
                        let request: NodeRequest = serde_json::from_value(request)
                            .context("failed to build reverse-link route request")?;
                        let response = self.start_route(request).await?;
                        route_response_with_plan(&response, plan)
                    }
                    route::RemoteUsePlan::Direct(local_peer) => {
                        self.ensure_peer_for_route(&args).await?;
                        let token = self.ensure_local_transport_token().await?;
                        let config = self.config.lock().await.clone();
                        let host_args =
                            route::remote_direct_host_args(&args, &config, local_peer, token)?;
                        let plan = remote_direct_route_plan(&args, &host_args.command, local_peer);
                        deploy::host(host_args, config).await?;
                        response_line(remote_direct_route_response(&args.target, plan))
                    }
                }
            }
        }
    }

    async fn ensure_peer_for_route(&self, args: &cli::RouteArgs) -> Result<()> {
        if self.peer_is_recorded(&args.target).await {
            return Ok(());
        }

        let (install_args, profile_name) = {
            let mut config = self.config.lock().await;
            let identity = config.ensure_node_identity()?;
            if config.daemon.control_endpoint.is_none() && config.daemon.control_listen.is_none() {
                config.daemon.control_endpoint = Some(self.control_endpoint.to_string());
            }
            if config.daemon.transport_listen.is_none() {
                config.daemon.transport_listen = self.transport;
            }
            if config.daemon.token.is_none() {
                config.ensure_daemon_token()?;
            }
            config.save_default()?;

            let mut install_args = route::install_args_from_route(args, &config)?;
            install_args.local_node_id = identity.node_id;
            install_args.local_node_name = identity.node_name;
            install_args.local_control_endpoint = Some(self.control_endpoint.to_string());
            install_args.local_transport = self.transport;
            install_args.persist = cli::PersistMode::Auto;
            (install_args, args.target.clone())
        };

        info!(target = %profile_name, "trying to adopt existing peer node through SSH descriptor");
        match deploy::refresh_remote_peer_descriptor(install_args.clone()).await {
            Ok(result) => {
                let mut config = self.config.lock().await;
                deploy::record_remote_descriptor_profile(&mut config, &profile_name, &result)?;
                return Ok(());
            }
            Err(err) => {
                info!(
                    target = %profile_name,
                    error = %err,
                    "peer descriptor refresh failed; falling back to SSH bootstrap"
                );
            }
        }

        info!(target = %profile_name, "bootstrapping peer node through SSH");
        let result = deploy::install_remote(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_install_profile(&mut config, &profile_name, &result)?;
        Ok(())
    }

    async fn ensure_local_transport_token(&self) -> Result<String> {
        let mut config = self.config.lock().await;
        let token = config.ensure_daemon_token()?;
        config.save_default()?;
        Ok(token)
    }
}

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
                .map(|id| format!("ssh_proxy down --route-id {id}"))
                .map(Value::from)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "health".to_string(),
            json!({
                "state": "accepted",
                "message": "route accepted; query `ssh_proxy status --json` for daemon-owned health"
            }),
        );
        object.insert(
            "job_id".to_string(),
            route_id
                .as_str()
                .map(|id| format!("route:{id}"))
                .map(Value::from)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "readiness".to_string(),
            accepted_readiness(route_id.as_str(), "route accepted"),
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
        .map(|id| format!("ssh_proxy down --target {target} --route-id {id}"))
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
        "job_id": route_id
            .as_str()
            .map(|id| format!("route:{id}"))
            .map(Value::from)
            .unwrap_or(Value::Null),
        "readiness": accepted_readiness(route_id.as_str(), "remote route intent accepted"),
        "health": {
            "state": "accepted",
            "message": "remote-owned route accepted; query the remote node for live health"
        },
        "plan": plan
    })
}

fn accepted_readiness(route_id: Option<&str>, message: &str) -> Value {
    json!({
        "state": "accepted",
        "phase": "starting",
        "retry_count": 0,
        "blocker": Value::Null,
        "next_action": "poll_routes",
        "managed_by": "current-daemon",
        "job_id": route_id.map(|id| format!("route:{id}")),
        "route_id": route_id,
        "message": message,
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
