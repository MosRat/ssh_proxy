use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

#[cfg(test)]
use crate::cli;

use super::{
    NodeManager, NodeRequest,
    jobs::{JobPhase, JobRecord, JobState},
    remote_setup, response_line,
    state::ProxySessionRecordExt,
};

mod apply_settings;
mod job_runner;
mod route_ready;
mod spec;
mod state_machine;
mod status;

use job_runner::job_for_phase;
#[cfg(test)]
use job_runner::{route_request_from_spec, validate_proxy_session_spec};
#[cfg(test)]
use spec::proxy_url_for_remote;
pub(crate) use spec::{ApplyPolicy, ProxySessionSpec, RemotePortPolicy, SshTargetSpec};

impl NodeManager {
    pub(super) async fn ensure_proxy_session(
        self: Arc<Self>,
        request: NodeRequest,
    ) -> Result<String> {
        let spec = request
            .proxy_session
            .ok_or_else(|| anyhow!("ensure_proxy_session requires proxy_session spec"))?;
        let job_id = spec.job_id();
        if let Some(existing) = self.jobs.get(&job_id).await {
            if state_machine::reusable_proxy_session_job(
                &existing,
                self.proxy_session_route_is_live(&spec).await,
            ) && self.proxy_session_matches_existing(&job_id, &spec).await
            {
                let session = self
                    .state
                    .upsert_session_from_job(&spec, &existing, None)
                    .await?;
                return status::accepted_response(&spec, &existing, session.to_value(), true);
            }
        }

        let job = JobRecord::new(job_id, "ensure_proxy_session")
            .with_target(spec.target.clone())
            .with_workspace(spec.workspace_id.clone())
            .with_route(spec.route_id())
            .with_remote_url(Some(spec.remote_url()))
            .transition(JobState::Queued, JobPhase::Queued, 0);
        let job = self.jobs.upsert(job, "proxy session accepted").await?;
        let session = self
            .state
            .upsert_session_from_job(&spec, &job, None)
            .await?;
        let manager = self.clone();
        let task_spec = spec.clone();
        let job_id = job.id.clone();
        tokio::spawn(async move {
            if let Err(err) = manager
                .clone()
                .run_proxy_session_job(task_spec.clone(), job_id.clone())
                .await
            {
                let failed = job_for_phase(&task_spec, &job_id, JobPhase::Failed, 100)
                    .failed(err.to_string(), Some("job_task_failed".to_string()));
                let _ = manager
                    .jobs
                    .upsert(failed, "proxy session job task failed")
                    .await;
            }
        });
        status::accepted_response(&spec, &job, session.to_value(), false)
    }

    async fn proxy_session_matches_existing(&self, job_id: &str, spec: &ProxySessionSpec) -> bool {
        let Some(session) = self.state.session_by_job(job_id).await else {
            return false;
        };
        match session.to_spec() {
            Ok(existing) => proxy_session_specs_match(&existing, spec),
            Err(_) => false,
        }
    }

    async fn proxy_session_route_is_live(&self, spec: &ProxySessionSpec) -> bool {
        let route_id = spec.route_id();
        let routes = self.routes.lock().await;
        routes
            .get(&route_id)
            .map(|task| !task.handle.is_finished())
            .unwrap_or(false)
    }

    pub(super) async fn proxy_session_status(&self, request: NodeRequest) -> Result<String> {
        let id = request
            .id
            .or_else(|| request.proxy_session.as_ref().map(ProxySessionSpec::job_id));
        let job = match id.as_deref() {
            Some(id) => self.jobs.get(id).await,
            None => None,
        };
        let session = match id.as_deref() {
            Some(id) => self.state.session_by_job(id).await,
            None => None,
        };
        let route_id = job
            .as_ref()
            .and_then(|job| job.route_id.clone())
            .or_else(|| session.as_ref().map(|session| session.route_id.clone()));
        let live_route = match route_id.as_deref() {
            Some(route_id) => self.live_route_status(route_id).await?,
            None => None,
        };
        let missing_healthy_route =
            matches!(job.as_ref().map(|job| job.state), Some(JobState::Healthy))
                && live_route.is_none()
                && route_id.is_some();
        let route = live_route
            .or_else(|| {
                if missing_healthy_route {
                    Some(status::missing_route(
                        route_id.clone(),
                        job.as_ref()
                            .and_then(|job| job.remote_url.clone())
                            .or_else(|| session.as_ref().map(|session| session.remote_url.clone())),
                    ))
                } else {
                    None
                }
            })
            .or_else(|| job.as_ref().map(status::route_from_job))
            .or_else(|| session.as_ref().and_then(|session| session.route.clone()))
            .unwrap_or(Value::Null);
        let health = (if missing_healthy_route {
            Some("starting")
        } else {
            None
        })
        .or_else(|| job.as_ref().map(state_machine::job_health))
        .or_else(|| session.as_ref().map(|session| session.health.as_str()))
        .unwrap_or("unknown");
        let ok = job.is_some() || session.is_some();
        response_line(json!({
            "ok": ok,
            "kind": "proxy_session_status",
            "daemon_api": "v0.3",
            "job": job.as_ref().map(JobRecord::to_value),
            "session": session.as_ref().map(|session| session.to_value()),
            "route": route,
            "remote_url": job.as_ref().and_then(|job| job.remote_url.clone())
                .or_else(|| session.as_ref().map(|session| session.remote_url.clone())),
            "remote_setup": session.as_ref().map(|session| serde_json::to_value(&session.remote_setup).unwrap_or(Value::Null)),
            "handoff_probe": session.as_ref().and_then(|session| session.handoff_probe.clone()),
            "health": health,
            "code": if ok { Value::Null } else { json!("not_found") },
        }))
    }

    async fn live_route_status(&self, route_id: &str) -> Result<Option<Value>> {
        let status = self.status_value().await?;
        Ok(status::find_route(&status, route_id))
    }

    pub(super) async fn proxy_session_down(&self, request: NodeRequest) -> Result<String> {
        let request_spec = request.proxy_session;
        let id = request
            .id
            .or_else(|| request_spec.as_ref().map(ProxySessionSpec::route_id))
            .ok_or_else(|| anyhow!("proxy_session_down requires id or proxy_session spec"))?;
        let cleanup_spec = match request_spec.clone() {
            Some(spec) => Some(spec),
            None => self
                .state
                .session_by_route(&id)
                .await
                .and_then(|record| record.to_spec().ok()),
        };
        let route_response = self.stop_route(NodeRequest::route_stop(id.clone())).await;
        let cleanup_response = match cleanup_spec.as_ref() {
            Some(spec) => {
                let config = {
                    let config = self.config.lock().await;
                    config.clone()
                };
                match remote_setup::cleanup_remote_settings(&config, spec, &spec.remote_url()).await
                {
                    Ok(()) => json!({
                        "ok": true,
                        "state": "cleaned",
                    }),
                    Err(err) => json!({
                        "ok": false,
                        "state": "failed",
                        "last_error": err.to_string(),
                        "next_action": "rerun_proxy_session_down"
                    }),
                }
            }
            None => json!({
                "ok": false,
                "state": "skipped",
                "last_error": "no proxy session spec was available for remote cleanup",
                "next_action": "rerun down with target/workspace while session state exists"
            }),
        };
        let job_id = request_spec
            .as_ref()
            .map(ProxySessionSpec::job_id)
            .or_else(|| cleanup_spec.as_ref().map(ProxySessionSpec::job_id))
            .unwrap_or_else(|| format!("proxy:{id}"));
        let job = JobRecord::new(job_id, "ensure_proxy_session")
            .with_route(id.clone())
            .transition(JobState::Cancelled, JobPhase::Cancelled, 100);
        let job = self.jobs.upsert(job, "proxy session stopped").await?;
        let session = self
            .state
            .cancel_session(
                &id,
                &job.id,
                route_response.as_ref().err().map(|err| err.to_string()),
            )
            .await?;
        response_line(json!({
            "ok": route_response.is_ok(),
            "kind": "proxy_session_down",
            "daemon_api": "v0.3",
            "route_id": id,
            "job": job.to_value(),
            "session": session.map(|session| session.to_value()),
            "route_stop": route_response.ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()),
            "remote_cleanup": cleanup_response,
        }))
    }

    pub(super) async fn reconcile_proxy_sessions(&self) -> Result<()> {
        for session in self.state.unfinished_sessions().await {
            let job = JobRecord::new(session.job_id.clone(), "ensure_proxy_session")
                .with_target(session.target.clone())
                .with_workspace(session.workspace_id.clone())
                .with_route(session.route_id.clone())
                .with_remote_url(Some(session.remote_url.clone()))
                .transition(JobState::WaitingRetry, JobPhase::Reconciling, 5)
                .with_next_action("rerun_ensure_proxy_session");
            self.jobs
                .upsert(
                    job,
                    "proxy session requires reconciliation after daemon restart",
                )
                .await?;
        }
        Ok(())
    }
}

fn proxy_session_specs_match(left: &ProxySessionSpec, right: &ProxySessionSpec) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    normalize_proxy_session_spec_for_live_reuse(&mut left);
    normalize_proxy_session_spec_for_live_reuse(&mut right);
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

fn normalize_proxy_session_spec_for_live_reuse(spec: &mut ProxySessionSpec) {
    if let Some(ssh) = spec.ssh.as_mut() {
        ssh.identity.clear();
    }
}

fn error_chain(err: &anyhow::Error) -> String {
    format!("{err:#}")
}

#[cfg(test)]
mod tests {
    use std::{net::IpAddr, path::PathBuf};

    use super::*;

    #[test]
    fn proxy_session_spec_derives_stable_ids() {
        let spec = ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: None,
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        assert_eq!(spec.route_id(), "v3-window-a");
        assert_eq!(spec.job_id(), "proxy:window-a");
        assert_eq!(spec.remote_url(), "http://127.0.0.1:17890/");
    }

    #[test]
    fn proxy_url_preserves_userinfo_and_suffix() {
        assert_eq!(
            proxy_url_for_remote("http://user:pass@127.0.0.1:10808/path", "127.0.0.1", 17890),
            "http://user:pass@127.0.0.1:17890/path",
        );
    }

    #[test]
    fn healthy_proxy_session_job_requires_live_route_for_reuse() {
        let healthy = JobRecord::new("proxy:window-a", "ensure_proxy_session").transition(
            JobState::Healthy,
            JobPhase::Healthy,
            100,
        );
        let running = JobRecord::new("proxy:window-a", "ensure_proxy_session").transition(
            JobState::Running,
            JobPhase::EnsurePeer,
            35,
        );

        assert!(!state_machine::reusable_proxy_session_job(&healthy, false));
        assert!(state_machine::reusable_proxy_session_job(&healthy, true));
        assert!(state_machine::reusable_proxy_session_job(&running, false));
    }

    #[test]
    fn route_request_uses_client_supplied_ssh_target() {
        let spec = ProxySessionSpec {
            target: "102".to_string(),
            workspace_id: Some("Window A".to_string()),
            ssh: Some(SshTargetSpec {
                host_name: Some("10.10.100.71".to_string()),
                user: Some("wenhongli".to_string()),
                port: Some(10022),
                identity: vec![PathBuf::from("C:/Users/whl/.ssh/id_rsa")],
                config: Some(PathBuf::from("C:/Users/whl/.ssh/config")),
                known_hosts: Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts")),
                jump: vec!["hub".to_string()],
                accept_new: true,
            }),
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };

        let request = route_request_from_spec(&spec);
        let route = request.route.expect("route args");

        assert_eq!(route.target, "102");
        assert_eq!(route.user.as_deref(), Some("wenhongli"));
        assert_eq!(route.ssh_port, Some(10022));
        assert_eq!(
            route.identity,
            vec![PathBuf::from("C:/Users/whl/.ssh/id_rsa")]
        );
        assert_eq!(
            route.config,
            Some(PathBuf::from("C:/Users/whl/.ssh/config"))
        );
        assert_eq!(
            route.known_hosts,
            Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts"))
        );
        assert_eq!(route.jump, vec!["hub"]);
        assert!(route.accept_new);
        assert_eq!(route.ssh_args, vec!["-o", "HostName=10.10.100.71"]);
    }

    #[test]
    fn proxy_session_reuse_ignores_identity_enrichment() {
        let mut existing = ProxySessionSpec {
            target: "125".to_string(),
            workspace_id: Some("wenhongli@172.18.116.125".to_string()),
            ssh: Some(SshTargetSpec {
                host_name: Some("172.18.116.125".to_string()),
                user: Some("wenhongli".to_string()),
                port: None,
                identity: Vec::new(),
                config: Some(PathBuf::from("C:/Users/whl/.ssh/config")),
                known_hosts: Some(PathBuf::from("C:/Users/whl/.ssh/known_hosts")),
                jump: Vec::new(),
                accept_new: true,
            }),
            workspace_paths: Vec::new(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        let mut enriched = existing.clone();
        enriched.ssh.as_mut().unwrap().identity = vec![
            PathBuf::from("C:/Users/whl/.ssh/id_rsa"),
            PathBuf::from("C:/Users/whl/.ssh/id_ed25519"),
        ];

        assert!(proxy_session_specs_match(&existing, &enriched));

        existing.remote_port_policy.preferred = 17891;
        assert!(!proxy_session_specs_match(&existing, &enriched));
    }

    #[test]
    fn validates_supported_local_proxy_urls() {
        let mut spec = ProxySessionSpec {
            target: "126".to_string(),
            workspace_id: None,
            ssh: None,
            workspace_paths: Vec::new(),
            local_proxy: "socks5h://user:pass@[::1]:1080/path".to_string(),
            remote_bind: "127.0.0.1".parse::<IpAddr>().unwrap(),
            remote_port_policy: RemotePortPolicy {
                preferred: 17890,
                auto_pick: true,
            },
            connect_mode: cli::RouteConnectMode::ReverseLink,
            apply_policy: ApplyPolicy::default(),
        };
        validate_proxy_session_spec(&spec).unwrap();

        spec.local_proxy = "https://127.0.0.1:10808/".to_string();
        assert!(validate_proxy_session_spec(&spec).is_err());

        spec.local_proxy = "http://127.0.0.1:abc/".to_string();
        assert!(validate_proxy_session_spec(&spec).is_err());
    }

    #[test]
    fn apply_settings_request_builds_spec_from_remote_url() {
        let request = NodeRequest::apply_remote_settings(
            "126".to_string(),
            "Window A".to_string(),
            "http://127.0.0.1:17890/".to_string(),
        );

        let spec = apply_settings::spec_from_apply_request(&request).unwrap();

        assert_eq!(spec.target, "126");
        assert_eq!(spec.workspace_id.as_deref(), Some("Window A"));
        assert_eq!(spec.remote_bind, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(spec.remote_port_policy.preferred, 17890);
    }

    #[test]
    fn remote_endpoint_parser_accepts_ipv6_urls() {
        let (host, port) = apply_settings::remote_endpoint_from_url("http://[::1]:17890/").unwrap();

        assert_eq!(host, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(port, 17890);
    }
}
