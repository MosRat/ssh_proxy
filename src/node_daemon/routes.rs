use std::{net::SocketAddr, sync::Arc};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::info;

use crate::{cli, controller, quic_native, ssh_native};

use super::{NodeManager, NodeRequest, response_line};

mod conflict_policy;
mod runtime_metadata;
mod store;
mod supervisor;
mod time_util;

use conflict_policy::{ensure_new_route_can_start, route_specs_match, route_task_matches};
use supervisor::spawn_route_supervisor;
use time_util::now_unix;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RouteStartOutcome {
    Started,
    ReusedExisting {
        persist: bool,
        upgraded_persist: bool,
    },
}

impl RouteStartOutcome {
    fn reused_existing(self) -> bool {
        matches!(self, Self::ReusedExisting { .. })
    }

    fn persist(self, requested: bool) -> bool {
        match self {
            Self::Started => requested,
            Self::ReusedExisting { persist, .. } => persist,
        }
    }

    fn should_save_route_store(self, requested: bool) -> bool {
        match self {
            Self::Started => requested,
            Self::ReusedExisting {
                upgraded_persist, ..
            } => upgraded_persist,
        }
    }
}

pub(super) struct RouteTask {
    pub(super) spec: RouteSpec,
    pub(super) direction: String,
    pub(super) detail: String,
    pub(super) listen: Option<SocketAddr>,
    pub(super) peer: Option<String>,
    pub(super) persist: bool,
    pub(super) created_at_unix: u64,
    pub(super) fallback_reason: Option<String>,
    pub(super) stats: Arc<Mutex<RouteStats>>,
    pub(super) link_state: Option<RouteLinkState>,
    pub(super) handle: JoinHandle<()>,
}

#[derive(Clone)]
pub(super) enum RouteLinkState {
    Spx(Arc<controller::SharedState>),
    QuicNative(Arc<quic_native::StateSlot>),
    SshNative(Arc<ssh_native::State>),
}

impl RouteLinkState {
    pub(super) async fn status_value(&self) -> serde_json::Value {
        match self {
            Self::Spx(state) => state.status_value().await,
            Self::QuicNative(slot) => slot.status_value().await,
            Self::SshNative(state) => state.status_value().await,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "direction", rename_all = "lowercase")]
pub(super) enum RouteSpec {
    Forward { proxy: cli::ProxyArgs },
    Reverse { reverse: cli::ReverseTaskArgs },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RouteStats {
    pub(super) state: String,
    pub(super) attempts: u64,
    pub(super) restart_count: u64,
    pub(super) last_error: Option<String>,
    pub(super) last_event: Option<String>,
    pub(super) started_at_unix: u64,
    pub(super) updated_at_unix: u64,
}

impl Default for RouteStats {
    fn default() -> Self {
        let now = now_unix();
        Self {
            state: "starting".to_string(),
            attempts: 0,
            restart_count: 0,
            last_error: None,
            last_event: Some("route task created".to_string()),
            started_at_unix: now,
            updated_at_unix: now,
        }
    }
}

impl NodeManager {
    pub(super) async fn start_route(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("route_start requires id"))?;
        let direction = request
            .direction
            .ok_or_else(|| anyhow!("route_start requires direction"))?;
        let spec = match direction.as_str() {
            "forward" => {
                let mut args = request
                    .proxy
                    .ok_or_else(|| anyhow!("forward route requires proxy args"))?;
                {
                    let config = self.config.lock().await;
                    config.apply_proxy_defaults(&mut args, None)?;
                }
                RouteSpec::Forward { proxy: args }
            }
            "reverse" => {
                let args = request
                    .reverse
                    .ok_or_else(|| anyhow!("reverse route requires reverse args"))?;
                RouteSpec::Reverse { reverse: args }
            }
            other => bail!("unknown route direction {other:?}"),
        };
        let persist = request.persist.unwrap_or(true);
        let route_direction = spec.direction().to_string();
        let detail = spec.detail();
        let listen = spec.listen();
        let peer = spec.peer().to_string();
        let fallback_reason = request.fallback_reason;
        let outcome = self
            .start_route_spec(
                id.clone(),
                spec,
                persist,
                now_unix(),
                fallback_reason.clone(),
            )
            .await?;
        if outcome.should_save_route_store(persist) {
            self.save_routes().await?;
        }
        response_line(json!({
            "ok": true,
            "message": if outcome.reused_existing() {
                format!("route {id:?} already running; reusing existing task")
            } else {
                format!("route {id:?} started")
            },
            "id": id.clone(),
            "reused_existing": outcome.reused_existing(),
            "owner": "local",
            "direction": route_direction,
            "detail": detail,
            "listen": listen.to_string(),
            "peer": peer,
            "persist": outcome.persist(persist),
            "fallback_reason": fallback_reason,
        }))
    }

    pub(super) async fn stop_route(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("route_stop requires id"))?;
        let mut routes = self.routes.lock().await;
        if let Some(task) = routes.remove(&id) {
            task.handle.abort();
            let persist = task.persist;
            drop(routes);
            if persist {
                self.save_routes().await?;
            }
            response_line(json!({
                "ok": true,
                "message": format!("route {id:?} stopped"),
                "removed_persistent": persist,
            }))
        } else {
            bail!("route {id:?} is not running");
        }
    }

    pub(super) async fn restart_route(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .ok_or_else(|| anyhow!("route_restart requires id"))?;
        let (spec, persist, created_at_unix, fallback_reason, handle) = {
            let mut routes = self.routes.lock().await;
            let Some(task) = routes.remove(&id) else {
                bail!("route {id:?} is not running");
            };
            (
                task.spec,
                task.persist,
                task.created_at_unix,
                task.fallback_reason,
                task.handle,
            )
        };
        handle.abort();
        let _ = handle.await;
        self.start_route_spec(id.clone(), spec, persist, created_at_unix, fallback_reason)
            .await?;
        if persist {
            self.save_routes().await?;
        }
        response_line(json!({
            "ok": true,
            "message": format!("route {id:?} restarted"),
            "persist": persist,
        }))
    }

    pub(super) async fn route_list_json(&self) -> Result<String> {
        let status = self.status_value().await?;
        response_line(json!({
                "ok": true,
                "route_store": status["route_store"].clone(),
                "route_autostart": status["route_autostart"].clone(),
                "routes": status["routes"].clone(),
        }))
    }

    pub(super) async fn start_route_spec(
        &self,
        id: String,
        spec: RouteSpec,
        persist: bool,
        created_at_unix: u64,
        fallback_reason: Option<String>,
    ) -> Result<RouteStartOutcome> {
        let direction = spec.direction().to_string();
        let detail = spec.detail();
        let listen = spec.listen();
        let peer = spec.peer().to_string();
        {
            let mut routes = self.routes.lock().await;
            routes.retain(|_, task| !task.handle.is_finished());
            if let Some(task) = routes.get_mut(&id) {
                if route_task_matches(task, &direction, listen, Some(&peer), &spec) {
                    let upgraded_persist = persist && !task.persist;
                    if upgraded_persist {
                        task.persist = true;
                    }
                    info!(route = %id, %direction, %listen, persist = task.persist, "route already registered; reusing existing task");
                    return Ok(RouteStartOutcome::ReusedExisting {
                        persist: task.persist,
                        upgraded_persist,
                    });
                }
                bail!("route {id:?} is already running with a different spec");
            }
            ensure_new_route_can_start(&routes, &direction, listen, Some(&peer), &spec)?;
        }
        let stats = Arc::new(Mutex::new(RouteStats::default()));
        let route_config = {
            let config = self.config.lock().await;
            config.clone()
        };
        let link_state = match &spec {
            RouteSpec::Forward { proxy } => match proxy.remote_transport {
                cli::RemoteTransport::SshNative => Some(RouteLinkState::SshNative(
                    ssh_native::State::new(proxy.clone()),
                )),
                cli::RemoteTransport::QuicNative => {
                    Some(RouteLinkState::QuicNative(quic_native::StateSlot::new()))
                }
                _ => Some(RouteLinkState::Spx(controller::shared_state(proxy))),
            },
            RouteSpec::Reverse { .. } => None,
        };
        let mut routes = self.routes.lock().await;
        routes.retain(|_, task| !task.handle.is_finished());
        if let Some(task) = routes.get_mut(&id) {
            if route_task_matches(task, &direction, listen, Some(&peer), &spec) {
                let upgraded_persist = persist && !task.persist;
                if upgraded_persist {
                    task.persist = true;
                }
                info!(route = %id, %direction, %listen, persist = task.persist, "route already registered during start race; reusing existing task");
                return Ok(RouteStartOutcome::ReusedExisting {
                    persist: task.persist,
                    upgraded_persist,
                });
            }
            bail!("route {id:?} is already running with a different spec");
        }
        ensure_new_route_can_start(&routes, &direction, listen, Some(&peer), &spec)?;
        let handle = spawn_route_supervisor(
            id.clone(),
            spec.clone(),
            route_config,
            stats.clone(),
            link_state.clone(),
        );
        routes.insert(
            id.clone(),
            RouteTask {
                spec,
                direction: direction.clone(),
                detail,
                listen: Some(listen),
                peer: Some(peer),
                persist,
                created_at_unix,
                fallback_reason,
                stats,
                link_state,
                handle,
            },
        );
        info!(route = %id, %direction, %listen, persist, "route registered");
        Ok(RouteStartOutcome::Started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_args() -> cli::ProxyArgs {
        cli::ProxyArgs {
            target: "peer".to_string(),
            listen: "127.0.0.1:18080".parse().unwrap(),
            tcp_target: None,
            ssh_args: Vec::new(),
            ssh_command: None,
            user: None,
            port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            deploy: cli::DeployMode::Auto,
            remote_os: cli::RemoteOs::Auto,
            remote_transport: cli::RemoteTransport::SshNative,
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
            remote_quic: None,
            allow_plain_tcp: false,
            remote_tls: None,
            remote_ca: None,
            remote_name: "localhost".to_string(),
            remote_client_cert: None,
            remote_client_key: None,
            remote_token: None,
            egress_proxy: None,
            reconnect_delay_secs: 5,
            reconnect_max_delay_secs: 60,
            connect_timeout_secs: 30,
            transport_pool_size: 4,
            pool_policy: Some("concurrent".to_string()),
            workload_hint: Some(cli::RouteWorkloadHint::Concurrent),
            quic_max_bidi_streams: crate::peer_transport::QUIC_MAX_BIDI_STREAMS,
            quic_stream_receive_window: crate::peer_transport::QUIC_STREAM_RECEIVE_WINDOW,
            quic_receive_window: crate::peer_transport::QUIC_RECEIVE_WINDOW,
            quic_keep_alive_interval_secs: crate::peer_transport::QUIC_KEEP_ALIVE_INTERVAL_SECS,
            quic_idle_timeout_secs: crate::peer_transport::QUIC_IDLE_TIMEOUT_SECS,
            ssh_session_pool_size: Some(2),
            ssh_session_pool_source: Some("implicit".to_string()),
            ssh_session_pool_reason: Some(
                "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes"
                    .to_string(),
            ),
            ssh_session_pool_warning: None,
            transport_pool_source: Some("implicit".to_string()),
            transport_pool_reason: Some("test pool".to_string()),
            transport_selection_source: Some("route-preflight".to_string()),
            transport_selection_reason: Some("test selection".to_string()),
            preflight_recommended_fallback: Some("ssh-native".to_string()),
            preflight_selected_reason: Some("test selected reason".to_string()),
            preflight_repair_hint: Some("test repair hint".to_string()),
            preflight_candidate_failures: vec![json!({
                "protocol": "tls-tcp",
                "status": "timeout",
            })],
            no_reconnect: false,
            control_listen: None,
        }
    }

    fn reverse_args() -> cli::ReverseTaskArgs {
        cli::ReverseTaskArgs {
            target: "125".to_string(),
            remote_listen: "127.0.0.1:17890".parse().unwrap(),
            tcp_target: None,
            ssh_args: vec!["-o".to_string(), "HostName=172.18.116.125".to_string()],
            user: Some("wenhongli".to_string()),
            port: None,
            identity: Vec::new(),
            config: Some("C:/Users/whl/.ssh/config".into()),
            known_hosts: Some("C:/Users/whl/.ssh/known_hosts".into()),
            accept_new: true,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            deploy: cli::DeployMode::Auto,
            remote_os: cli::RemoteOs::Auto,
            egress_proxy: Some("http://127.0.0.1:10808/".to_string()),
            reconnect_delay_secs: 5,
            reconnect_max_delay_secs: 60,
            connect_timeout_secs: 30,
            transport_pool_source: Some("fixed".to_string()),
            transport_pool_reason: Some("reverse-link test".to_string()),
            no_reconnect: false,
        }
    }

    #[test]
    fn reverse_route_match_ignores_identity_enrichment() {
        let mut left = reverse_args();
        let mut right = left.clone();
        right.identity = vec![
            "C:/Users/whl/.ssh/id_rsa".into(),
            "C:/Users/whl/.ssh/id_ed25519".into(),
        ];

        assert!(route_specs_match(
            &RouteSpec::Reverse {
                reverse: left.clone()
            },
            &RouteSpec::Reverse {
                reverse: right.clone()
            },
        ));

        left.remote_listen = "127.0.0.1:17891".parse().unwrap();
        assert!(!route_specs_match(
            &RouteSpec::Reverse { reverse: left },
            &RouteSpec::Reverse { reverse: right },
        ));
    }

    #[test]
    fn forward_runtime_metadata_exposes_preflight_decision_chain() {
        let spec = RouteSpec::Forward {
            proxy: proxy_args(),
        };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["selected_transport"], "ssh-native");
        assert_eq!(runtime["transport_selection_source"], "route-preflight");
        assert_eq!(
            runtime["connection_decision"]["selected_transport"],
            "ssh-native"
        );
        assert_eq!(runtime["connection_decision"]["source"], "route-preflight");
        assert_eq!(runtime["connection_decision"]["reason"], "test selection");
        assert_eq!(
            runtime["connection_decision"]["requires_external_ssh"],
            false
        );
        assert_eq!(runtime["ssh_mode"], "native-direct-tcpip");
        assert_eq!(runtime["ssh_data_plane_reason"], "simple_egress");
        assert!(
            runtime["ssh_mode_reason"]
                .as_str()
                .expect("ssh mode reason")
                .contains("simple SSH-only local egress")
        );
        assert_eq!(runtime["ssh_session_pool_size"], 2);
        assert_eq!(runtime["pool_policy"], "concurrent");
        assert_eq!(runtime["workload_hint"], "concurrent");
        assert_eq!(runtime["ssh_session_pool_source"], "implicit");
        assert_eq!(
            runtime["ssh_session_pool_reason"],
            "implicit ssh-native two-session default for multi-flow SOCKS/HTTP proxy routes"
        );
        assert_eq!(runtime["preflight"]["recommended_fallback"], "ssh-native");
        assert_eq!(
            runtime["preflight"]["selected_reason"],
            "test selected reason"
        );
        assert_eq!(runtime["preflight"]["repair_hint"], "test repair hint");
        assert_eq!(
            runtime["preflight"]["candidate_failures"][0]["protocol"],
            "tls-tcp"
        );
        assert_eq!(runtime["decision_chain"]["topology"]["class"], "ssh-only");
        assert_eq!(
            runtime["decision_chain"]["policy"]["selection_source"],
            "route-preflight"
        );
        assert_eq!(
            runtime["decision_chain"]["policy"]["ssh_data_plane_reason"],
            "simple_egress"
        );
        assert_eq!(
            runtime["decision_chain"]["selected_transport"],
            "ssh-native"
        );
        assert_eq!(
            runtime["decision_chain"]["preflight"]["repair_hint"],
            "test repair hint"
        );
    }

    #[test]
    fn forward_runtime_metadata_exposes_large_ssh_pool_warning() {
        let mut proxy = proxy_args();
        proxy.ssh_session_pool_size = Some(8);
        proxy.ssh_session_pool_source = Some("command-line".to_string());
        proxy.ssh_session_pool_reason = Some("loaded from --ssh-session-pool-size".to_string());
        proxy.ssh_session_pool_warning = Some(
            "ssh-native session pools above 2 can lose to handshake and scheduling overhead; benchmark before relying on this explicit value"
                .to_string(),
        );
        let spec = RouteSpec::Forward { proxy };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["ssh_session_pool_size"], 8);
        assert_eq!(runtime["ssh_session_pool_source"], "command-line");
        assert!(
            runtime["ssh_session_pool_warning"]
                .as_str()
                .expect("pool warning")
                .contains("above 2")
        );
    }

    #[test]
    fn forward_runtime_metadata_marks_exec_as_external_compatibility() {
        let mut proxy = proxy_args();
        proxy.remote_transport = cli::RemoteTransport::Exec;
        let spec = RouteSpec::Forward { proxy };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["selected_transport"], "ssh-exec");
        assert_eq!(
            runtime["connection_decision"]["requires_external_ssh"],
            true
        );
        assert_eq!(
            runtime["connection_decision"]["details"]["ssh_data_plane_reason"],
            "ssh_exec_compatibility"
        );
    }

    #[test]
    fn forward_runtime_metadata_explains_spx_over_ssh_mode() {
        let mut proxy = proxy_args();
        proxy.remote_transport = cli::RemoteTransport::Tcp;
        let spec = RouteSpec::Forward { proxy };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["selected_transport"], "ssh-direct-tcpip");
        assert_eq!(
            runtime["connection_decision"]["endpoint"],
            "ssh-direct-tcpip://127.0.0.1:19080"
        );
        assert_eq!(runtime["ssh_mode"], "spx-over-ssh-direct");
        assert_eq!(runtime["ssh_data_plane_reason"], "daemon_policy_required");
        assert!(
            runtime["ssh_mode_reason"]
                .as_str()
                .expect("ssh mode reason")
                .contains("token auth")
        );
        assert!(
            runtime["ssh_mode_reason"]
                .as_str()
                .expect("ssh mode reason")
                .contains("route restore")
        );
    }

    #[test]
    fn forward_runtime_metadata_reports_direct_transport_policy() {
        let mut proxy = proxy_args();
        proxy.remote_transport = cli::RemoteTransport::TlsTcp;
        proxy.remote_tls = Some("127.0.0.1:19443".parse().unwrap());
        let spec = RouteSpec::Forward { proxy };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["direct_transport_policy"], "production_direct");
        assert_eq!(
            runtime["connection_decision"]["endpoint"],
            "tls-tcp://127.0.0.1:19443"
        );
        assert!(
            runtime["direct_transport_policy_reason"]
                .as_str()
                .expect("policy reason")
                .contains("production direct baseline")
        );
        assert_eq!(
            runtime["decision_chain"]["policy"]["direct_transport_policy"],
            "production_direct"
        );
        assert_eq!(runtime["tls_peer_auth_mode"], "server_auth");

        let mut proxy = proxy_args();
        proxy.remote_transport = cli::RemoteTransport::PlainTcp;
        let spec = RouteSpec::Forward { proxy };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["direct_transport_policy"], "lab_baseline");
        assert_eq!(
            runtime["connection_decision"]["endpoint"],
            "plain-tcp://127.0.0.1:19080"
        );
        assert!(
            runtime["direct_transport_policy_reason"]
                .as_str()
                .expect("policy reason")
                .contains("lab or explicitly trusted baseline")
        );
        assert!(runtime["tls_peer_auth_mode"].is_null());
    }

    #[test]
    fn reverse_runtime_metadata_uses_shared_connection_decision_shape() {
        let spec = RouteSpec::Reverse {
            reverse: reverse_args(),
        };

        let runtime = spec.runtime_metadata();

        assert_eq!(runtime["selected_transport"], "ssh-reverse-link");
        assert_eq!(
            runtime["connection_decision"]["selected_transport"],
            "ssh-reverse-link"
        );
        assert_eq!(runtime["connection_decision"]["source"], "topology");
        assert_eq!(
            runtime["connection_decision"]["requires_external_ssh"],
            false
        );
    }
}
