use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use tracing::info;

use crate::{cli, config, deploy, peer_transport, route};

use super::{NodeManager, NodeRequest, NodeResponse, control_protocol, response_line};

mod bootstrap;
mod compatibility;
mod reconciliation;
mod registry;
mod route_intent;

impl NodeManager {
    pub(super) async fn peers_json(&self) -> Result<String> {
        let config = self.config.lock().await;
        response_line(registry::peers_response(&config))
    }

    pub(super) async fn forget_peer(&self, request: NodeRequest) -> Result<String> {
        let alias = request
            .alias
            .ok_or_else(|| anyhow!("peer_forget requires alias"))?;
        let mut config = self.config.lock().await;
        registry::remove_peer(&mut config, &alias)?;
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
                route_intent::route_response_with_plan(&response, plan)
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
                        route_intent::route_response_with_plan(&response, plan)
                    }
                    route::RemoteUsePlan::Direct(local_peer) => {
                        let token = self.ensure_local_transport_token().await?;
                        let config = self.config.lock().await.clone();
                        let host_args =
                            route::remote_direct_host_args(&args, &config, local_peer, token)?;
                        let plan = route_intent::remote_direct_route_plan(
                            &args,
                            &host_args.command,
                            local_peer,
                        );
                        deploy::host(host_args, config).await?;
                        response_line(route_intent::remote_direct_route_response(
                            &args.target,
                            plan,
                        ))
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
        compatibility::attach_saved_peer_compatibility(&mut plan, &args, &config);
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

            let mut install_args = bootstrap::install_args_for_bootstrap(
                &args,
                identity,
                self.control_endpoint.to_string(),
                self.transport,
            );
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args.persist = cli::PersistMode::Auto;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "bootstrapping peer node through SSH");
        let result = deploy::install_remote(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_install_profile(&mut config, &alias, &result)?;
        response_line(bootstrap::bootstrap_response(&alias, &result))
    }

    async fn refresh_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
            let config = self.config.lock().await;
            let mut install_args = bootstrap::install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "refreshing peer descriptor through SSH");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_descriptor_profile(&mut config, &alias, &result)?;
        response_line(bootstrap::refresh_response(&alias, &result))
    }

    async fn diff_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let (config_snapshot, install_args) = {
            let config = self.config.lock().await;
            let mut install_args = bootstrap::install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            (config.clone(), install_args)
        };

        info!(target = %install_args.target, alias = %alias, "diffing peer descriptor through SSH");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        response_line(reconciliation::build_peer_diff(
            &alias,
            &config_snapshot,
            &result,
        ))
    }

    async fn reconcile_peer_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let (config_snapshot, install_args) = {
            let config = self.config.lock().await;
            let mut install_args = bootstrap::install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            (config.clone(), install_args)
        };

        info!(target = %install_args.target, alias = %alias, "reconciling peer descriptor through SSH without mutating local records");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        response_line(reconciliation::build_peer_reconcile(
            &alias,
            &config_snapshot,
            &result,
        ))
    }

    async fn check_peer_version_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
            let config = self.config.lock().await;
            let mut install_args = bootstrap::install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "checking peer protocol versions through SSH");
        let result = deploy::refresh_remote_peer_descriptor(install_args).await?;
        response_line(compatibility::build_peer_version_check(&alias, &result))
    }

    async fn rotate_peer_token_from_args(&self, args: cli::PeerBootstrapArgs) -> Result<String> {
        let alias = args.alias.clone().unwrap_or_else(|| args.target.clone());
        let install_args = {
            let config = self.config.lock().await;
            let mut install_args = bootstrap::install_args_from_bootstrap(&args);
            config.apply_install_defaults(&mut install_args, Some(&alias))?;
            install_args
        };

        info!(target = %install_args.target, alias = %alias, "rotating peer daemon token through SSH");
        let result = deploy::rotate_remote_peer_token(install_args).await?;
        let mut config = self.config.lock().await;
        deploy::record_remote_token_rotation_profile(&mut config, &alias, &result)?;
        response_line(bootstrap::token_rotation_response(&alias, &result))
    }

    async fn peer_is_recorded(&self, target: &str) -> bool {
        let config = self.config.lock().await;
        registry::peer_is_route_ready(&config, target)
    }

    async fn ensure_local_transport_token(&self) -> Result<String> {
        let mut config = self.config.lock().await;
        let token = config.ensure_daemon_token()?;
        config.save_default()?;
        Ok(token)
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

        let diff = reconciliation::build_peer_diff(alias, &config, &result);
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

        let reconcile = reconciliation::build_peer_reconcile(alias, &config, &result);

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

        let check = compatibility::build_peer_version_check("edge", &result);
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

        let check = compatibility::build_saved_peer_version_check("edge", &peer);

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

        let check = compatibility::build_peer_version_check("edge", &result);
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

        let output = route_intent::route_response_with_plan(&response, plan).unwrap();
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

        let remote_url = route_intent::remote_proxy_url_from_plan(&plan, &listen).unwrap();

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

        let value = route_intent::remote_direct_route_response("edge", plan);

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
