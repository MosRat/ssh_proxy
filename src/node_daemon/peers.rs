use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tracing::info;

use crate::{cli, config, deploy, peer_transport, route};

use super::{NodeManager, NodeRequest, NodeResponse, control_protocol, response_line};

impl NodeManager {
    pub(super) async fn peers_json(&self) -> Result<String> {
        let config = self.config.lock().await;
        let mut peers = config.peers.iter().collect::<Vec<_>>();
        peers.sort_by(|(left, _), (right, _)| left.cmp(right));
        let peers = peers
            .into_iter()
            .map(|(alias, peer)| {
                json!({
                    "alias": alias,
                    "node_id": peer.node_id,
                    "node_name": peer.node_name,
                    "service_instance_id": peer.service_instance_id,
                    "version": peer.version,
                    "control_api_version": peer.control_api_version,
                    "peer_protocol_version": peer.peer_protocol_version,
                    "features": peer.features,
                    "os": peer.os,
                    "arch": peer.arch,
                    "os_user": peer.os_user,
                    "data_dir": peer.data_dir,
                    "target": peer.target,
                    "trust": peer.trust,
                    "remote_path": peer.remote_path,
                    "control_endpoint": peer.control_endpoint,
                    "transport": peer.transport.map(|addr| addr.to_string()),
                    "tls_transport": peer.tls_transport.map(|addr| addr.to_string()),
                    "quic_transport": peer.quic_transport.map(|addr| addr.to_string()),
                    "transport_protocols": peer.known_transport_protocols(),
                    "auth": {
                        "token": peer.token.is_some(),
                        "token_metadata": peer.token_metadata.clone(),
                        "token_generation": peer.token_metadata.as_ref().map(|metadata| metadata.generation),
                        "tls_server_cert_fingerprint": peer.tls_server_cert_fingerprint,
                        "tls_client_ca_fingerprint": peer.tls_client_ca_fingerprint,
                    },
                    "compatibility": build_saved_peer_version_check(alias, peer),
                    "last_seen_unix": peer.last_seen_unix,
                })
            })
            .collect::<Vec<_>>();
        response_line(json!({
                "ok": true,
                "node_id": config.identity.node_id,
                "node_name": config.identity.node_name,
                "peers": peers,
        }))
    }

    pub(super) async fn forget_peer(&self, request: NodeRequest) -> Result<String> {
        let alias = request
            .alias
            .ok_or_else(|| anyhow!("peer_forget requires alias"))?;
        let mut config = self.config.lock().await;
        if config.peers.remove(&alias).is_none() {
            bail!("peer {alias:?} is not recorded");
        }
        config.save_default()?;
        NodeResponse::ok_message(format!("peer {alias:?} forgotten")).to_line()
    }

    pub(super) async fn bootstrap_peer(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .bootstrap
            .ok_or_else(|| anyhow!("peer_bootstrap requires bootstrap args"))?;
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        if !args.force && self.peer_is_recorded(&alias).await {
            return response_line(json!({
                    "ok": true,
                    "message": format!("peer {alias:?} already recorded"),
                    "alias": alias,
                    "changed": false
            }));
        }
        self.bootstrap_peer_from_args(args).await
    }

    pub(super) async fn refresh_peer(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .bootstrap
            .ok_or_else(|| anyhow!("peer_refresh requires bootstrap args"))?;
        self.refresh_peer_from_args(args).await
    }

    pub(super) async fn diff_peer(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .bootstrap
            .ok_or_else(|| anyhow!("peer_diff requires bootstrap args"))?;
        self.diff_peer_from_args(args).await
    }

    pub(super) async fn reconcile_peer(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .bootstrap
            .ok_or_else(|| anyhow!("peer_reconcile requires bootstrap args"))?;
        self.reconcile_peer_from_args(args).await
    }

    pub(super) async fn check_peer_version(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .bootstrap
            .ok_or_else(|| anyhow!("peer_check_version requires bootstrap args"))?;
        self.check_peer_version_from_args(args).await
    }

    pub(super) async fn rotate_peer_token(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .bootstrap
            .ok_or_else(|| anyhow!("peer_rotate_token requires bootstrap args"))?;
        self.rotate_peer_token_from_args(args).await
    }

    pub(super) async fn record_report(&self, request: NodeRequest) -> Result<String> {
        let node = request.node.unwrap_or_else(|| "unknown".to_string());
        let status = request.status.unwrap_or(Value::Null);
        self.peer_reports.lock().await.insert(node.clone(), status);
        NodeResponse::ok_message(format!("report accepted from {node}")).to_line()
    }

    pub(super) async fn handle_route_intent(&self, request: NodeRequest) -> Result<String> {
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
                self.ensure_peer_for_route(&args).await?;
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

    pub(super) async fn handle_route_plan(&self, request: NodeRequest) -> Result<String> {
        let args = request
            .route
            .ok_or_else(|| anyhow!("route_plan requires route args"))?;
        let config = self.config.lock().await.clone();
        let mut plan = route::explain_plan(&args, &config).await?;
        attach_saved_peer_compatibility(&mut plan, &args, &config);
        response_line(json!({
            "ok": true,
            "message": "route plan generated",
            "plan": plan,
        }))
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

    async fn bootstrap_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
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

            let mut install_args = cli::InstallRemoteArgs {
                target: args.target.clone(),
                ssh_args: args.ssh_args.clone(),
                ssh_command: None,
                user: args.user.clone(),
                port: args.port,
                identity: args.identity.clone(),
                config: args.config.clone(),
                known_hosts: args.known_hosts.clone(),
                accept_new: args.accept_new,
                insecure_ignore_host_key: args.insecure_ignore_host_key,
                jump: args.jump.clone(),
                remote_path: args.remote_path.clone(),
                remote_bin: args.remote_bin.clone(),
                remote_os: args.remote_os,
                remote_token: args.remote_token.clone(),
                remote_tcp: args.remote_tcp,
                remote_control: args.remote_control,
                local_node_id: identity.node_id,
                local_node_name: identity.node_name,
                local_control_endpoint: Some(self.control_endpoint.to_string()),
                local_transport: self.transport,
                remote_node_id: None,
                remote_node_name: None,
                remote_tls_transport: None,
                remote_quic_transport: None,
                remote_tls_cert: None,
                remote_tls_key: None,
                remote_tls_client_ca: None,
                persist: cli::PersistMode::Auto,
            };
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args.persist = cli::PersistMode::Auto;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "bootstrapping peer node through SSH");
        let result = deploy::install_remote(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_install_profile(&mut config, &alias, &result)?;
        response_line(json!({
                "ok": true,
                "message": format!("peer {alias:?} bootstrapped"),
                "alias": alias,
                "target": result.target,
                "node_id": result.remote_node_id,
                "node_name": result.remote_node_name,
                "remote_path": result.remote_path,
                "remote_tcp": result.remote_tcp.to_string(),
                "remote_control": result.remote_control.to_string(),
                "remote_tls_transport": result.remote_tls_transport.map(|addr| addr.to_string()),
                "remote_quic_transport": result.remote_quic_transport.map(|addr| addr.to_string()),
                "changed": true
        }))
    }

    async fn refresh_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
            let config = self.config.lock().await;
            let mut install_args = cli::InstallRemoteArgs {
                target: args.target.clone(),
                ssh_args: args.ssh_args.clone(),
                ssh_command: None,
                user: args.user.clone(),
                port: args.port,
                identity: args.identity.clone(),
                config: args.config.clone(),
                known_hosts: args.known_hosts.clone(),
                accept_new: args.accept_new,
                insecure_ignore_host_key: args.insecure_ignore_host_key,
                jump: args.jump.clone(),
                remote_path: args.remote_path.clone(),
                remote_bin: args.remote_bin.clone(),
                remote_os: args.remote_os,
                remote_token: args.remote_token.clone(),
                remote_tcp: args.remote_tcp,
                remote_control: args.remote_control,
                local_node_id: None,
                local_node_name: None,
                local_control_endpoint: None,
                local_transport: None,
                remote_node_id: None,
                remote_node_name: None,
                remote_tls_transport: None,
                remote_quic_transport: None,
                remote_tls_cert: None,
                remote_tls_key: None,
                remote_tls_client_ca: None,
                persist: cli::PersistMode::None,
            };
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "refreshing peer descriptor through SSH");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_descriptor_profile(&mut config, &alias, &result)?;
        response_line(json!({
                "ok": true,
                "message": format!("peer {alias:?} refreshed"),
                "alias": alias,
                "target": result.target,
                "node_id": result.descriptor.get("node_id").cloned().unwrap_or(Value::Null),
                "node_name": result.descriptor.get("node_name").cloned().unwrap_or(Value::Null),
                "version": result.descriptor.get("version").cloned().unwrap_or(Value::Null),
                "control_api_version": result.descriptor.get("control_api_version").cloned().unwrap_or(Value::Null),
                "peer_protocol_version": result.descriptor.get("peer_protocol_version").cloned().unwrap_or(Value::Null),
                "transport_protocols": result.descriptor.get("transport_protocols").cloned().unwrap_or(Value::Null),
                "changed": true
        }))
    }

    async fn diff_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let (config_snapshot, install_args) = {
            let config = self.config.lock().await;
            let mut install_args = peer_install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            (config.clone(), install_args)
        };

        info!(target = %install_args.target, alias = %alias, "diffing peer descriptor through SSH");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        response_line(build_peer_diff(&alias, &config_snapshot, &result))
    }

    async fn reconcile_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let (config_snapshot, install_args) = {
            let config = self.config.lock().await;
            let mut install_args = peer_install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            (config.clone(), install_args)
        };

        info!(target = %install_args.target, alias = %alias, "reconciling peer descriptor through SSH without mutating local records");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        response_line(build_peer_reconcile(&alias, &config_snapshot, &result))
    }

    async fn check_peer_version_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
            let config = self.config.lock().await;
            let mut install_args = peer_install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "checking peer protocol versions through SSH");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        response_line(build_peer_version_check(&alias, &result))
    }

    async fn rotate_peer_token_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
            let config = self.config.lock().await;
            let mut install_args = peer_install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "rotating peer daemon token through SSH");
        let result = deploy::rotate_remote_peer_token(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_token_rotation_profile(&mut config, &alias, &result)?;
        response_line(json!({
                "ok": true,
                "message": format!("peer {alias:?} token rotated"),
                "alias": alias,
                "target": result.target,
                "node_id": result
                    .descriptor
                    .as_ref()
                    .and_then(|descriptor| descriptor.get("node_id"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "node_name": result
                    .descriptor
                    .as_ref()
                    .and_then(|descriptor| descriptor.get("node_name"))
                    .cloned()
                    .unwrap_or(Value::Null),
                "token_metadata": result.token_metadata,
                "remote_response": result.response,
                "changed": true
        }))
    }

    async fn peer_is_recorded(&self, target: &str) -> bool {
        let config = self.config.lock().await;
        config.peers.get(target).is_some_and(|peer| {
            peer.remote_path.is_some()
                && peer.control_endpoint.is_some()
                && (peer.transport.is_some()
                    || peer.tls_transport.is_some()
                    || peer.quic_transport.is_some())
        })
    }

    async fn ensure_local_transport_token(&self) -> Result<String> {
        let mut config = self.config.lock().await;
        let token = config.ensure_daemon_token()?;
        config.save_default()?;
        Ok(token)
    }
}

fn peer_install_args_from_bootstrap(args: &cli::PeerBootstrapArgs) -> cli::InstallRemoteArgs {
    cli::InstallRemoteArgs {
        target: args.target.clone(),
        ssh_args: args.ssh_args.clone(),
        ssh_command: None,
        user: args.user.clone(),
        port: args.port,
        identity: args.identity.clone(),
        config: args.config.clone(),
        known_hosts: args.known_hosts.clone(),
        accept_new: args.accept_new,
        insecure_ignore_host_key: args.insecure_ignore_host_key,
        jump: args.jump.clone(),
        remote_path: args.remote_path.clone(),
        remote_bin: args.remote_bin.clone(),
        remote_os: args.remote_os,
        remote_token: args.remote_token.clone(),
        remote_tcp: args.remote_tcp,
        remote_control: args.remote_control,
        local_node_id: None,
        local_node_name: None,
        local_control_endpoint: None,
        local_transport: None,
        remote_node_id: None,
        remote_node_name: None,
        remote_tls_transport: None,
        remote_quic_transport: None,
        remote_tls_cert: None,
        remote_tls_key: None,
        remote_tls_client_ca: None,
        persist: cli::PersistMode::None,
    }
}

fn build_peer_diff(
    alias: &str,
    config: &config::AppConfig,
    result: &deploy::RemoteDescriptorResult,
) -> Value {
    let local_peer = config.peers.get(alias);
    let local_profile = config.profiles.get(alias);
    let local = local_peer_summary(alias, local_peer, local_profile);
    let remote = remote_descriptor_summary(result);
    let mut diffs = Vec::new();

    if local_peer.is_none() {
        diffs.push(json!({
            "field": "peer.recorded",
            "local": false,
            "remote": true,
            "action": "peer-refresh"
        }));
    }
    if local_profile.is_none() {
        diffs.push(json!({
            "field": "profile.recorded",
            "local": false,
            "remote": true,
            "action": "peer-refresh"
        }));
    }

    push_diff(
        &mut diffs,
        "peer.target",
        local
            .pointer("/peer/target")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("target").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.node_id",
        local
            .pointer("/peer/node_id")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("node_id").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.node_name",
        local
            .pointer("/peer/node_name")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("node_name").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.service_instance_id",
        local
            .pointer("/peer/service_instance_id")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .get("service_instance_id")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.version",
        local
            .pointer("/peer/version")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("version").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.remote_path",
        local
            .pointer("/peer/remote_path")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("remote_path").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.control_endpoint",
        local
            .pointer("/peer/control_endpoint")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .get("control_endpoint")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.transport",
        local
            .pointer("/peer/transport")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.tls_transport",
        local
            .pointer("/peer/tls_transport")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("tls_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.quic_transport",
        local
            .pointer("/peer/quic_transport")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("quic_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "peer.transport_protocols",
        local
            .pointer("/peer/transport_protocols")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .get("transport_protocols")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "auth.token",
        local.pointer("/auth/token").cloned().unwrap_or(Value::Null),
        remote
            .pointer("/auth/token")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );
    push_diff(
        &mut diffs,
        "auth.token_scope",
        local
            .pointer("/auth/token_scope")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/token_scope")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );
    push_diff(
        &mut diffs,
        "auth.token_generation",
        local
            .pointer("/auth/token_generation")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/token_generation")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );
    push_diff(
        &mut diffs,
        "auth.tls_server_cert_fingerprint",
        local
            .pointer("/auth/tls_server_cert_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/tls_server_cert_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "auth.tls_client_ca_fingerprint",
        local
            .pointer("/auth/tls_client_ca_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/tls_client_ca_fingerprint")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_control",
        local
            .pointer("/profile/remote_control")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("remote_control").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_tcp",
        local
            .pointer("/profile/remote_tcp")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_tls",
        local
            .pointer("/profile/remote_tls")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("tls_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_quic",
        local
            .pointer("/profile/remote_quic")
            .cloned()
            .unwrap_or(Value::Null),
        remote.get("quic_transport").cloned().unwrap_or(Value::Null),
        "peer-refresh",
    );
    push_diff(
        &mut diffs,
        "profile.remote_token",
        local
            .pointer("/profile/remote_token")
            .cloned()
            .unwrap_or(Value::Null),
        remote
            .pointer("/auth/token")
            .cloned()
            .unwrap_or(Value::Null),
        "peer-rotate-token",
    );

    let changed = !diffs.is_empty();
    let next_action = next_peer_diff_action(&diffs);
    json!({
        "ok": true,
        "kind": "peer_diff",
        "alias": alias,
        "target": result.target,
        "changed": changed,
        "local": local,
        "remote": remote,
        "diffs": diffs,
        "next_action": next_action,
    })
}

fn build_peer_reconcile(
    alias: &str,
    config: &config::AppConfig,
    result: &deploy::RemoteDescriptorResult,
) -> Value {
    let local_peer = config.peers.get(alias);
    let local_profile = config.profiles.get(alias);
    let local = local_peer_summary(alias, local_peer, local_profile);
    let remote = remote_descriptor_summary(result);
    let diff = build_peer_diff(alias, config, result);
    let version = build_peer_version_check(alias, result);
    let mut issues = Vec::new();

    if local_peer.is_none() || local_profile.is_none() {
        issues.push(json!({
            "code": "missing_local_record",
            "severity": "warning",
            "message": "remote daemon descriptor exists but the local peer/profile record is incomplete",
            "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
        }));
    }

    let local_node_id = local
        .pointer("/peer/node_id")
        .cloned()
        .unwrap_or(Value::Null);
    let remote_node_id = remote.get("node_id").cloned().unwrap_or(Value::Null);
    if !local_node_id.is_null() && !remote_node_id.is_null() && local_node_id != remote_node_id {
        issues.push(json!({
            "code": "stale_remote_record",
            "severity": "warning",
            "message": "local peer identity points at a different remote node id",
            "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
        }));
    }

    if version.get("status").and_then(Value::as_str) != Some("compatible") {
        let compatible = version
            .get("compatible")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        issues.push(json!({
            "code": "version_mismatch",
            "severity": if compatible { "warning" } else { "error" },
            "message": if compatible {
                "remote version differs but advertised protocols/features still allow fallback"
            } else {
                "remote version or feature set is incompatible with this binary"
            },
            "fallback_when_compatible": compatible,
            "repair_command": version.get("next_action").cloned().unwrap_or(Value::String("peer-bootstrap --force".to_string())),
        }));
    }

    let local_token = local.pointer("/auth/token").cloned().unwrap_or(Value::Null);
    let remote_token = remote
        .pointer("/auth/token")
        .cloned()
        .unwrap_or(Value::Null);
    let local_generation = local
        .pointer("/auth/token_generation")
        .cloned()
        .unwrap_or(Value::Null);
    let remote_generation = remote
        .pointer("/auth/token_generation")
        .cloned()
        .unwrap_or(Value::Null);
    if local_token != remote_token
        || (!local_generation.is_null()
            && !remote_generation.is_null()
            && local_generation != remote_generation)
    {
        issues.push(json!({
            "code": "token_mismatch",
            "severity": "warning",
            "message": "local record and remote descriptor disagree about token presence or token generation",
            "repair_command": format!("ssh_proxy node control peer-rotate-token {target} --alias {alias}", target = result.target),
        }));
    }

    let cert_pairs = [
        (
            "tls_server_cert_fingerprint",
            local.pointer("/auth/tls_server_cert_fingerprint"),
            remote.pointer("/auth/tls_server_cert_fingerprint"),
        ),
        (
            "tls_client_ca_fingerprint",
            local.pointer("/auth/tls_client_ca_fingerprint"),
            remote.pointer("/auth/tls_client_ca_fingerprint"),
        ),
    ];
    for (field, local_value, remote_value) in cert_pairs {
        if let (Some(local_value), Some(remote_value)) = (local_value, remote_value)
            && !local_value.is_null()
            && !remote_value.is_null()
            && local_value != remote_value
        {
            issues.push(json!({
                "code": "certificate_mismatch",
                "field": field,
                "severity": "warning",
                "message": "local certificate fingerprint differs from the remote descriptor",
                "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
            }));
        }
    }

    for field in ["service_instance_id", "os_user", "data_dir"] {
        let local_value = local
            .pointer(&format!("/peer/{field}"))
            .cloned()
            .unwrap_or(Value::Null);
        let remote_value = remote.get(field).cloned().unwrap_or(Value::Null);
        if !local_value.is_null() && !remote_value.is_null() && local_value != remote_value {
            issues.push(json!({
                "code": "ownership_mismatch",
                "field": field,
                "severity": "warning",
                "message": "local peer record points at a different daemon owner or service instance",
                "repair_command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
            }));
        }
    }

    let adoption_plan = if local_peer.is_none() || local_profile.is_none() {
        json!({
            "needed": true,
            "mode": "adopt-existing-daemon",
            "command": format!("ssh_proxy node control peer-refresh {target} --alias {alias}", target = result.target),
            "will_replace_auth_material": false,
            "note": "dry-run only; refresh/adoption must be invoked explicitly"
        })
    } else {
        json!({
            "needed": false,
            "mode": "none",
            "will_replace_auth_material": false
        })
    };

    json!({
        "ok": true,
        "kind": "peer_reconcile",
        "alias": alias,
        "target": result.target,
        "changed": false,
        "dry_run": true,
        "explicit_repair_required": true,
        "local": local,
        "remote": remote,
        "diff": diff,
        "version": version,
        "issues": issues,
        "adoption_plan": adoption_plan,
        "repair_commands": issues
            .iter()
            .filter_map(|issue| issue.get("repair_command").cloned())
            .collect::<Vec<_>>(),
    })
}

fn local_peer_summary(
    alias: &str,
    peer: Option<&config::PeerRecord>,
    profile: Option<&config::ProxyProfile>,
) -> Value {
    json!({
        "alias": alias,
        "peer": {
            "recorded": peer.is_some(),
            "target": peer.and_then(|peer| peer.target.clone()),
            "node_id": peer.and_then(|peer| peer.node_id.clone()),
            "node_name": peer.and_then(|peer| peer.node_name.clone()),
            "service_instance_id": peer.and_then(|peer| peer.service_instance_id.clone()),
            "trust": peer.and_then(|peer| peer.trust.clone()),
            "remote_path": peer.and_then(|peer| peer.remote_path.clone()),
            "control_endpoint": peer.and_then(|peer| peer.control_endpoint.clone()),
            "transport": peer.and_then(|peer| peer.transport).map(|addr| addr.to_string()),
            "tls_transport": peer.and_then(|peer| peer.tls_transport).map(|addr| addr.to_string()),
            "quic_transport": peer.and_then(|peer| peer.quic_transport).map(|addr| addr.to_string()),
            "transport_protocols": peer.map(config::PeerRecord::known_transport_protocols).unwrap_or_default(),
            "version": peer.and_then(|peer| peer.version.clone()),
            "control_api_version": peer.and_then(|peer| peer.control_api_version),
            "peer_protocol_version": peer.and_then(|peer| peer.peer_protocol_version),
            "features": peer.map(|peer| peer.features.clone()).unwrap_or_default(),
            "os_user": peer.and_then(|peer| peer.os_user.clone()),
            "data_dir": peer.and_then(|peer| peer.data_dir.clone()),
            "last_seen_unix": peer.and_then(|peer| peer.last_seen_unix),
        },
        "profile": {
            "recorded": profile.is_some(),
            "target": profile.and_then(|profile| profile.target.clone()),
            "remote_path": profile.and_then(|profile| profile.remote_path.clone()),
            "remote_control": profile.and_then(|profile| profile.remote_control).map(|addr| addr.to_string()),
            "remote_tcp": profile.and_then(|profile| profile.remote_tcp).map(|addr| addr.to_string()),
            "remote_tls": profile.and_then(|profile| profile.remote_tls).map(|addr| addr.to_string()),
            "remote_quic": profile.and_then(|profile| profile.remote_quic).map(|addr| addr.to_string()),
            "remote_transport": profile.and_then(|profile| profile.remote_transport.clone()),
            "remote_token": profile.and_then(|profile| profile.remote_token.as_ref()).is_some(),
        },
        "auth": {
            "token": peer.and_then(|peer| peer.token.as_ref()).is_some(),
            "token_metadata": peer.and_then(|peer| peer.token_metadata.clone()),
            "token_scope": peer
                .and_then(|peer| peer.token_metadata.as_ref())
                .map(|metadata| metadata.scope.clone()),
            "token_generation": peer
                .and_then(|peer| peer.token_metadata.as_ref())
                .map(|metadata| metadata.generation),
            "tls_server_cert_fingerprint": peer.and_then(|peer| peer.tls_server_cert_fingerprint.clone()),
            "tls_client_ca_fingerprint": peer.and_then(|peer| peer.tls_client_ca_fingerprint.clone()),
        }
    })
}

fn remote_descriptor_summary(result: &deploy::RemoteDescriptorResult) -> Value {
    let descriptor = &result.descriptor;
    let control_endpoint = descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    let token_metadata = descriptor.pointer("/auth/token_metadata").cloned();
    let token_scope = token_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("scope"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    json!({
        "target": result.target,
        "node_id": descriptor.get("node_id").cloned().unwrap_or(Value::Null),
        "node_name": descriptor.get("node_name").cloned().unwrap_or(Value::Null),
        "service_instance_id": descriptor.get("service_instance_id").cloned().unwrap_or(Value::Null),
        "version": descriptor.get("version").cloned().unwrap_or(Value::Null),
        "control_api_version": descriptor.get("control_api_version").cloned().unwrap_or(Value::Null),
        "peer_protocol_version": descriptor.get("peer_protocol_version").cloned().unwrap_or(Value::Null),
        "features": descriptor.get("features").cloned().unwrap_or(Value::Array(Vec::new())),
        "os_user": descriptor.get("os_user").cloned().unwrap_or(Value::Null),
        "data_dir": descriptor.get("data_dir").cloned().unwrap_or(Value::Null),
        "remote_path": result.remote_path,
        "control_endpoint": control_endpoint,
        "remote_control": result.remote_control.to_string(),
        "transport": result.remote_tcp.to_string(),
        "tls_transport": result.remote_tls_transport.map(|addr| addr.to_string()),
        "quic_transport": result.remote_quic_transport.map(|addr| addr.to_string()),
        "transport_protocols": descriptor_protocols(descriptor).unwrap_or_else(|| {
            let mut protocols = Vec::new();
            if result.remote_quic_transport.is_some() {
                protocols.push("quic".to_string());
            }
            if result.remote_tls_transport.is_some() {
                protocols.push("tls-tcp".to_string());
            }
            protocols.push("plain-tcp".to_string());
            protocols
        }),
        "auth": {
            "token": descriptor
                .pointer("/auth/control_token")
                .and_then(Value::as_bool)
                .unwrap_or(result.remote_token.is_some()),
            "token_metadata": token_metadata,
            "token_scope": token_scope,
            "token_generation": descriptor
                .pointer("/auth/token_generation")
                .cloned()
                .or_else(|| descriptor.pointer("/auth/token_metadata/generation").cloned())
                .unwrap_or(Value::Null),
            "tls_server_cert_fingerprint": descriptor
                .pointer("/auth/tls_server_cert_fingerprint")
                .cloned()
                .unwrap_or(Value::Null),
            "tls_client_ca_fingerprint": descriptor
                .pointer("/auth/tls_client_ca_fingerprint")
                .cloned()
                .unwrap_or(Value::Null),
        }
    })
}

fn descriptor_protocols(descriptor: &Value) -> Option<Vec<String>> {
    let protocols = descriptor
        .get("transport_protocols")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!protocols.is_empty()).then_some(protocols)
}

fn push_diff(diffs: &mut Vec<Value>, field: &str, local: Value, remote: Value, action: &str) {
    if local != remote {
        diffs.push(json!({
            "field": field,
            "local": local,
            "remote": remote,
            "action": action
        }));
    }
}

fn next_peer_diff_action(diffs: &[Value]) -> &'static str {
    if diffs.is_empty() {
        return "none";
    }
    let has_refresh = diffs
        .iter()
        .any(|diff| diff.get("action").and_then(Value::as_str) == Some("peer-refresh"));
    if has_refresh {
        "peer-refresh"
    } else {
        "peer-rotate-token"
    }
}

fn build_peer_version_check(alias: &str, result: &deploy::RemoteDescriptorResult) -> Value {
    let descriptor = &result.descriptor;
    let local_version = env!("CARGO_PKG_VERSION");
    let remote_version = descriptor.get("version").and_then(Value::as_str);
    let local_control = control_protocol::NODE_CONTROL_VERSION;
    let remote_control = descriptor.get("control_api_version").and_then(value_to_u16);
    let local_peer = peer_transport::PEER_VERSION;
    let remote_peer = descriptor
        .get("peer_protocol_version")
        .and_then(value_to_u16);
    let local_features = peer_transport::default_features();
    let remote_features = string_array_field(descriptor, "features");
    let missing_features = local_features
        .iter()
        .filter(|feature| !remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();
    let common_features = local_features
        .iter()
        .filter(|feature| remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();

    let mut checks = Vec::new();
    checks.push(control_api_check(local_control, remote_control));
    checks.push(peer_protocol_check(local_peer, remote_peer));
    checks.push(feature_check(
        &local_features,
        &remote_features,
        &missing_features,
    ));
    checks.push(binary_version_check(local_version, remote_version));

    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    let next_action = version_next_action(
        remote_version,
        remote_control,
        remote_peer,
        local_control,
        local_peer,
        &missing_features,
    );
    let status = version_status(compatible, local_version, remote_version, next_action);

    json!({
        "ok": true,
        "kind": "peer_version_check",
        "alias": alias,
        "target": result.target,
        "compatible": compatible,
        "status": status,
        "local": {
            "version": local_version,
            "control_api_version": local_control,
            "peer_protocol_version": local_peer,
            "features": local_features,
        },
        "remote": {
            "version": remote_version,
            "control_api_version": remote_control,
            "peer_protocol_version": remote_peer,
            "features": remote_features,
            "common_features": common_features,
            "missing_features": missing_features,
            "os": descriptor.get("os").cloned().unwrap_or(Value::Null),
            "arch": descriptor.get("arch").cloned().unwrap_or(Value::Null),
        },
        "checks": checks,
        "next_action": next_action,
    })
}

fn build_saved_peer_version_check(alias: &str, peer: &config::PeerRecord) -> Value {
    let local_version = env!("CARGO_PKG_VERSION");
    let remote_version = peer.version.as_deref();
    let local_control = control_protocol::NODE_CONTROL_VERSION;
    let remote_control = peer.control_api_version;
    let local_peer = peer_transport::PEER_VERSION;
    let remote_peer = peer.peer_protocol_version;
    let local_features = peer_transport::default_features();
    let remote_features = peer.features.clone();
    let missing_features = local_features
        .iter()
        .filter(|feature| !remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();
    let common_features = local_features
        .iter()
        .filter(|feature| remote_features.contains(*feature))
        .cloned()
        .collect::<Vec<_>>();

    let checks = vec![
        control_api_check(local_control, remote_control),
        peer_protocol_check(local_peer, remote_peer),
        feature_check(&local_features, &remote_features, &missing_features),
        binary_version_check(local_version, remote_version),
    ];
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    let next_action = version_next_action(
        remote_version,
        remote_control,
        remote_peer,
        local_control,
        local_peer,
        &missing_features,
    );
    let status = version_status(compatible, local_version, remote_version, next_action);

    json!({
        "kind": "saved_peer_version_check",
        "alias": alias,
        "recorded": true,
        "fresh": false,
        "compatible": compatible,
        "status": status,
        "local": {
            "version": local_version,
            "control_api_version": local_control,
            "peer_protocol_version": local_peer,
            "features": local_features,
        },
        "remote": {
            "version": remote_version,
            "control_api_version": remote_control,
            "peer_protocol_version": remote_peer,
            "features": remote_features,
            "common_features": common_features,
            "missing_features": missing_features,
            "os": peer.os,
            "arch": peer.arch,
        },
        "checks": checks,
        "next_action": next_action,
    })
}

fn attach_saved_peer_compatibility(
    plan: &mut Value,
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) {
    let compatibility = config
        .peers
        .get(&args.target)
        .map(|peer| build_saved_peer_version_check(&args.target, peer))
        .unwrap_or_else(|| {
            json!({
                "kind": "saved_peer_version_check",
                "alias": args.target,
                "recorded": false,
                "compatible": false,
                "status": "unrecorded",
                "checks": [],
                "next_action": "peer-bootstrap",
                "message": "peer is not recorded locally; route start will try descriptor adoption then SSH bootstrap"
            })
        });
    if let Value::Object(object) = plan {
        object.insert("peer_compatibility".to_string(), compatibility);
    }
}

fn control_api_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote <= local => json!({
            "name": "control_api_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote control API is supported"
        }),
        Some(remote) => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote control API is newer than this binary supports"
        }),
        None => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "remote descriptor does not advertise a control API version"
        }),
    }
}

fn peer_protocol_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote == local => json!({
            "name": "peer_protocol_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote peer data protocol matches"
        }),
        Some(remote) if remote > local => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote peer data protocol is newer than this binary supports"
        }),
        Some(remote) => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote peer data protocol is older than this binary requires"
        }),
        None => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "remote descriptor does not advertise a peer data protocol version"
        }),
    }
}

fn feature_check(local: &[String], remote: &[String], missing: &[String]) -> Value {
    if missing.is_empty() {
        json!({
            "name": "features",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote advertises all locally required data-plane features"
        })
    } else {
        json!({
            "name": "features",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote is missing required data-plane features",
            "missing": missing,
        })
    }
}

fn binary_version_check(local: &str, remote: Option<&str>) -> Value {
    match remote.and_then(|remote| compare_dotted_versions(local, remote)) {
        Some(std::cmp::Ordering::Equal) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "local and remote binaries report the same package version"
        }),
        Some(std::cmp::Ordering::Greater) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "remote binary is older; bootstrap with --force to align versions"
        }),
        Some(std::cmp::Ordering::Less) => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "remote binary is newer; consider upgrading the local binary"
        }),
        None => json!({
            "name": "binary_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "warning",
            "message": "binary version could not be compared"
        }),
    }
}

fn version_next_action(
    remote_version: Option<&str>,
    remote_control: Option<u16>,
    remote_peer: Option<u16>,
    local_control: u16,
    local_peer: u16,
    missing_features: &[String],
) -> &'static str {
    if remote_control.is_some_and(|remote| remote > local_control)
        || remote_peer.is_some_and(|remote| remote > local_peer)
    {
        return "upgrade-local";
    }
    if remote_control.is_none()
        || remote_peer.is_none()
        || remote_peer.is_some_and(|remote| remote < local_peer)
        || !missing_features.is_empty()
    {
        return "peer-bootstrap --force";
    }
    match remote_version
        .and_then(|remote| compare_dotted_versions(env!("CARGO_PKG_VERSION"), remote))
    {
        Some(std::cmp::Ordering::Greater) => "peer-bootstrap --force",
        Some(std::cmp::Ordering::Less) => "upgrade-local",
        _ => "none",
    }
}

fn version_status(
    compatible: bool,
    local_version: &str,
    remote_version: Option<&str>,
    next_action: &str,
) -> &'static str {
    if !compatible {
        return "incompatible";
    }
    match remote_version.and_then(|remote| compare_dotted_versions(local_version, remote)) {
        Some(std::cmp::Ordering::Equal) => "compatible",
        Some(std::cmp::Ordering::Greater) if next_action == "peer-bootstrap --force" => {
            "compatible-upgrade-remote"
        }
        Some(std::cmp::Ordering::Less) if next_action == "upgrade-local" => {
            "compatible-upgrade-local"
        }
        _ => "compatible-version-unknown",
    }
}

fn value_to_u16(value: &Value) -> Option<u16> {
    value.as_u64().and_then(|value| u16::try_from(value).ok())
}

fn string_array_field(value: &Value, field: &str) -> Vec<String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn compare_dotted_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let left = parse_dotted_version(left)?;
    let right = parse_dotted_version(right)?;
    Some(left.cmp(&right))
}

fn parse_dotted_version(value: &str) -> Option<Vec<u64>> {
    let core = value.split_once('-').map(|(core, _)| core).unwrap_or(value);
    let parts = core
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (!parts.is_empty()).then_some(parts)
}

fn route_response_with_plan(response: &str, plan: Value) -> Result<String> {
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
                "state": "starting",
                "message": "route accepted; query `ssh_proxy node control routes` for live health"
            }),
        );
        object.insert("plan".to_string(), plan);
    }
    response_line(value)
}

fn remote_proxy_url_from_plan(plan: &Value, remote_listen: &Value) -> Option<Value> {
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

fn remote_direct_route_response(target: &str, plan: Value) -> Value {
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

fn remote_direct_route_plan(
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

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use serde_json::json;

    use super::*;

    #[test]
    fn peer_diff_reports_record_and_token_drift_without_secrets() {
        let alias = "edge";
        let mut config = config::AppConfig::default();
        config.profiles.insert(
            alias.to_string(),
            config::ProxyProfile {
                target: Some("edge.example".to_string()),
                remote_path: Some("/opt/old/ssh_proxy".to_string()),
                remote_control: Some("127.0.0.1:29081".parse().unwrap()),
                remote_tcp: Some("127.0.0.1:29080".parse().unwrap()),
                remote_token: None,
                ..Default::default()
            },
        );
        config.record_peer(
            alias,
            config::PeerRecord {
                node_id: Some("old-node".to_string()),
                node_name: Some("old-name".to_string()),
                target: Some("edge.example".to_string()),
                remote_path: Some("/opt/old/ssh_proxy".to_string()),
                control_endpoint: Some("tcp://127.0.0.1:29081".to_string()),
                transport: Some("127.0.0.1:29080".parse().unwrap()),
                transport_protocols: vec!["plain-tcp".to_string()],
                token: None,
                ..Default::default()
            },
        );

        let result = deploy::RemoteDescriptorResult {
            target: "edge.example".to_string(),
            remote_path: "/opt/ssh_proxy/ssh_proxy".to_string(),
            remote_control: "127.0.0.1:19081".parse::<SocketAddr>().unwrap(),
            remote_tcp: "127.0.0.1:19080".parse::<SocketAddr>().unwrap(),
            remote_tls_transport: Some("127.0.0.1:19443".parse().unwrap()),
            remote_quic_transport: None,
            remote_token: Some("secret-token".to_string()),
            descriptor: json!({
                "ok": true,
                "node_id": "new-node",
                "node_name": "new-name",
                "version": "0.2.0",
                "control_api_version": 1,
                "peer_protocol_version": 1,
                "transport_protocols": ["tls-tcp", "plain-tcp"],
                "endpoints": {
                    "control": "tcp://127.0.0.1:19081"
                },
                "auth": {
                    "control_token": true,
                    "token_metadata": {
                        "created_at_unix": 42,
                        "rotated_at_unix": null,
                        "scope": "peer-control-transport",
                        "expires_at_unix": null
                    }
                }
            }),
        };

        let diff = build_peer_diff(alias, &config, &result);
        assert_eq!(diff["ok"], true);
        assert_eq!(diff["changed"], true);
        assert_eq!(diff["next_action"], "peer-refresh");
        assert_eq!(diff["local"]["auth"]["token"], false);
        assert_eq!(diff["remote"]["auth"]["token"], true);
        assert!(!diff.to_string().contains("secret-token"));

        let fields = diff["diffs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|diff| diff["field"].as_str())
            .collect::<Vec<_>>();
        assert!(fields.contains(&"peer.node_id"));
        assert!(fields.contains(&"peer.remote_path"));
        assert!(fields.contains(&"auth.token"));
    }

    #[test]
    fn peer_reconcile_reports_adoption_and_mismatches_without_mutation() {
        let alias = "edge";
        let config = config::AppConfig::default();
        let result = deploy::RemoteDescriptorResult {
            target: "edge.example".to_string(),
            remote_path: "/opt/ssh_proxy/ssh_proxy".to_string(),
            remote_control: "127.0.0.1:19081".parse::<SocketAddr>().unwrap(),
            remote_tcp: "127.0.0.1:19080".parse::<SocketAddr>().unwrap(),
            remote_tls_transport: Some("127.0.0.1:19443".parse().unwrap()),
            remote_quic_transport: None,
            remote_token: Some("secret-token".to_string()),
            descriptor: json!({
                "ok": true,
                "node_id": "remote-node",
                "node_name": "remote-name",
                "service_instance_id": "remote-node@alice:tcp://127.0.0.1:19081",
                "version": "0.0.1",
                "control_api_version": control_protocol::NODE_CONTROL_VERSION,
                "peer_protocol_version": peer_transport::PEER_VERSION,
                "features": peer_transport::default_features(),
                "os_user": "alice",
                "data_dir": "/home/alice/.ssh_proxy",
                "endpoints": {
                    "control": "tcp://127.0.0.1:19081",
                    "transport": "127.0.0.1:19080",
                    "tls_transport": "127.0.0.1:19443"
                },
                "auth": {
                    "control_token": true,
                    "token_generation": 3,
                    "tls_server_cert_fingerprint": "sha256:remote-cert"
                }
            }),
        };

        let reconcile = build_peer_reconcile(alias, &config, &result);

        assert_eq!(reconcile["ok"], true);
        assert_eq!(reconcile["kind"], "peer_reconcile");
        assert_eq!(reconcile["dry_run"], true);
        assert_eq!(reconcile["changed"], false);
        assert_eq!(reconcile["adoption_plan"]["needed"], true);
        assert!(reconcile.to_string().contains("missing_local_record"));
        assert!(reconcile.to_string().contains("version_mismatch"));
        assert!(!reconcile.to_string().contains("secret-token"));
    }

    #[test]
    fn peer_version_check_reports_upgrade_direction() {
        let result = deploy::RemoteDescriptorResult {
            target: "edge.example".to_string(),
            remote_path: "/opt/ssh_proxy/ssh_proxy".to_string(),
            remote_control: "127.0.0.1:19081".parse::<SocketAddr>().unwrap(),
            remote_tcp: "127.0.0.1:19080".parse::<SocketAddr>().unwrap(),
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_token: None,
            descriptor: json!({
                "ok": true,
                "version": "0.0.1",
                "os": "linux",
                "arch": "x86_64",
                "control_api_version": control_protocol::NODE_CONTROL_VERSION,
                "peer_protocol_version": peer_transport::PEER_VERSION,
                "features": peer_transport::default_features(),
            }),
        };

        let check = build_peer_version_check("edge", &result);
        assert_eq!(check["ok"], true);
        assert_eq!(check["compatible"], true);
        assert_eq!(check["next_action"], "peer-bootstrap --force");
        assert_eq!(check["status"], "compatible-upgrade-remote");
    }

    #[test]
    fn saved_peer_version_check_uses_recorded_metadata() {
        let peer = config::PeerRecord {
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            control_api_version: Some(control_protocol::NODE_CONTROL_VERSION),
            peer_protocol_version: Some(peer_transport::PEER_VERSION),
            features: peer_transport::default_features(),
            os: Some("linux".to_string()),
            arch: Some("x86_64".to_string()),
            ..Default::default()
        };

        let check = build_saved_peer_version_check("edge", &peer);

        assert_eq!(check["kind"], "saved_peer_version_check");
        assert_eq!(check["recorded"], true);
        assert_eq!(check["compatible"], true);
        assert_eq!(check["next_action"], "none");
        assert_eq!(check["remote"]["os"], "linux");
    }

    #[test]
    fn peer_version_check_rejects_future_peer_protocol() {
        let result = deploy::RemoteDescriptorResult {
            target: "edge.example".to_string(),
            remote_path: "/opt/ssh_proxy/ssh_proxy".to_string(),
            remote_control: "127.0.0.1:19081".parse::<SocketAddr>().unwrap(),
            remote_tcp: "127.0.0.1:19080".parse::<SocketAddr>().unwrap(),
            remote_tls_transport: None,
            remote_quic_transport: None,
            remote_token: None,
            descriptor: json!({
                "ok": true,
                "version": env!("CARGO_PKG_VERSION"),
                "control_api_version": control_protocol::NODE_CONTROL_VERSION,
                "peer_protocol_version": peer_transport::PEER_VERSION + 1,
                "features": peer_transport::default_features(),
            }),
        };

        let check = build_peer_version_check("edge", &result);
        assert_eq!(check["compatible"], false);
        assert_eq!(check["status"], "incompatible");
        assert_eq!(check["next_action"], "upgrade-local");
    }

    #[test]
    fn route_response_with_plan_adds_plugin_fields() {
        let response = serde_json::json!({
            "ok": true,
            "message": "route accepted",
            "id": "vscode-remote-proxy-edge",
            "listen": "127.0.0.1:17890",
            "fallback_reason": null
        })
        .to_string();
        let plan = serde_json::json!({
            "route_id": "vscode-remote-proxy-edge",
            "mode": "reverse-link",
            "selected_transport": "ssh-reverse-link",
            "listener": { "listen": "127.0.0.1:17890" },
            "egress": { "upstream_proxy": "http://127.0.0.1:18080" },
            "fallback_reason": "ssh-only topology"
        });

        let output = route_response_with_plan(&response, plan).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["route_id"], "vscode-remote-proxy-edge");
        assert_eq!(value["owner"], "local");
        assert_eq!(value["selected_transport"], "ssh-reverse-link");
        assert_eq!(value["connect_mode"], "reverse-link");
        assert_eq!(value["listen"], "127.0.0.1:17890");
        assert_eq!(value["remote_listen"], "127.0.0.1:17890");
        assert_eq!(value["remote_url"], "http://127.0.0.1:17890");
        assert_eq!(
            value["cleanup_command"],
            "ssh_proxy node control stop-route vscode-remote-proxy-edge"
        );
        assert_eq!(value["health"]["state"], "starting");
    }

    #[test]
    fn remote_proxy_url_from_plan_preserves_auth_and_suffix() {
        let plan = serde_json::json!({
            "egress": {
                "upstream_proxy": "http://demo-user:demo-credential@127.0.0.1:18080/proxy"
            }
        });
        let listen = serde_json::json!("127.0.0.1:17890");

        let remote_url = remote_proxy_url_from_plan(&plan, &listen).unwrap();

        assert_eq!(
            remote_url,
            "http://demo-user:demo-credential@127.0.0.1:17890/proxy"
        );
    }

    #[test]
    fn remote_direct_route_response_adds_plugin_fields() {
        let plan = serde_json::json!({
            "route_id": "vscode-remote-proxy-edge",
            "selected_transport": "tls-tcp",
            "listener": { "listen": "127.0.0.1:17890" },
            "egress": { "upstream_proxy": "socks5h://127.0.0.1:18080/" },
            "fallback_reason": null
        });

        let value = remote_direct_route_response("edge", plan);

        assert_eq!(value["route_id"], "vscode-remote-proxy-edge");
        assert_eq!(value["owner"], "remote");
        assert_eq!(value["connect_mode"], "direct");
        assert_eq!(value["selected_transport"], "tls-tcp");
        assert_eq!(value["listen"], "127.0.0.1:17890");
        assert_eq!(value["remote_listen"], "127.0.0.1:17890");
        assert_eq!(value["remote_url"], "socks5h://127.0.0.1:17890/");
        assert_eq!(
            value["cleanup_command"],
            "ssh_proxy host edge node-stop-route vscode-remote-proxy-edge"
        );
        assert_eq!(value["health"]["state"], "accepted");
    }
}
