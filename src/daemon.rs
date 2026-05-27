use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{cli, config, control_socket, node_daemon, route, service};

pub async fn daemon(args: cli::DaemonArgs, config: config::AppConfig) -> Result<()> {
    match args.command {
        cli::DaemonCommand::Install { elevate, no_copy } => {
            service::run(
                service_args(
                    args.scope,
                    args.json,
                    elevate,
                    no_copy,
                    cli::ServiceCommand::Install,
                ),
                config,
            )
            .await
        }
        cli::DaemonCommand::Uninstall => {
            service::run(
                service_args(
                    args.scope,
                    args.json,
                    false,
                    false,
                    cli::ServiceCommand::Uninstall,
                ),
                config,
            )
            .await
        }
        cli::DaemonCommand::Start => {
            service::run(
                service_args(
                    args.scope,
                    args.json,
                    false,
                    false,
                    cli::ServiceCommand::Start,
                ),
                config,
            )
            .await
        }
        cli::DaemonCommand::Stop => {
            service::run(
                service_args(
                    args.scope,
                    args.json,
                    false,
                    false,
                    cli::ServiceCommand::Stop,
                ),
                config,
            )
            .await
        }
        cli::DaemonCommand::Status => {
            status(
                cli::StatusArgs {
                    target: None,
                    workspace: None,
                    endpoint: control_socket::default_endpoint_string(),
                    token: None,
                    json: args.json,
                },
                config,
            )
            .await
        }
        cli::DaemonCommand::Update { source } => print_json(
            args.json,
            json!({
                "ok": true,
                "kind": "daemon_update",
                "daemon_api": "v0.3",
                "job": job_status(
                    "self-update:pending",
                    "self_update",
                    "accepted",
                    "queued",
                    "daemon self-update is represented as an allowlisted job in v0.3",
                ),
                "source": source.map(|path| path.display().to_string()),
                "requires_daemon": true,
            }),
        ),
        cli::DaemonCommand::Serve(node_args) => {
            node_daemon::run(
                cli::NodeArgs {
                    command: cli::NodeCommand::Daemon(node_args),
                },
                config,
            )
            .await
        }
    }
}

pub async fn up(args: cli::UpArgs, config: config::AppConfig) -> Result<()> {
    let route_args = route_args_from_up(&args);
    let request = route::route_intent_request(route_args);
    request_daemon_or_report(
        &args.endpoint,
        args.token.as_deref(),
        &config,
        request,
        args.json,
        || {
            json!({
                "kind": "proxy_session",
                "daemon_api": "v0.3",
                "spec": proxy_session_spec(
                    &args.target,
                    args.workspace.as_deref(),
                    &args.local_proxy,
                    &args.remote_bind.to_string(),
                    args.remote_port,
                ),
                "job": job_status(
                    &format!("proxy:{}", session_key(args.route_key())),
                    "ensure_proxy_session",
                    "blocked",
                    "daemon_unavailable",
                    "install or start the ssh_proxy daemon, then retry this proxy session",
                ),
            })
        },
    )
    .await
}

pub async fn down(args: cli::DownArgs, config: config::AppConfig) -> Result<()> {
    let id = match args.route_id.clone().or_else(|| {
        args.workspace
            .as_deref()
            .or(args.target.as_deref())
            .map(route_id_for_key)
    }) {
        Some(id) => id,
        None => bail!("down requires --route-id, --workspace, or --target"),
    };
    let request = node_daemon::NodeRequest::route_stop(id.clone())
        .to_value()
        .context("failed to encode route stop request")?;
    request_daemon_or_report(
        &args.endpoint,
        args.token.as_deref(),
        &config,
        request,
        args.json,
        || {
            json!({
                "ok": false,
                "kind": "proxy_session_down",
                "daemon_api": "v0.3",
                "route_id": id,
                "code": "daemon_unavailable",
                "requires_daemon": true,
                "requires_elevation": true,
            })
        },
    )
    .await
}

pub async fn status(args: cli::StatusArgs, config: config::AppConfig) -> Result<()> {
    let request = node_daemon::NodeRequest::command("status")
        .to_value()
        .context("failed to encode status request")?;
    request_daemon_or_report(
        &args.endpoint,
        args.token.as_deref(),
        &config,
        request,
        args.json,
        || {
            json!({
                "ok": false,
                "kind": "daemon_status",
                "daemon_api": "v0.3",
                "version": env!("CARGO_PKG_VERSION"),
                "target": args.target,
                "workspace": args.workspace,
                "health": "unavailable",
                "code": "daemon_unavailable",
                "requires_elevation": true,
                "next_action": "daemon install --scope system",
            })
        },
    )
    .await
}

pub async fn events(args: cli::EventsArgs, config: config::AppConfig) -> Result<()> {
    let request = node_daemon::NodeRequest::command("jobs")
        .to_value()
        .context("failed to encode jobs request")?;
    request_daemon_or_report(
        &args.endpoint,
        args.token.as_deref(),
        &config,
        request,
        args.json,
        || {
            json!({
                "ok": false,
                "kind": "daemon_events",
                "daemon_api": "v0.3",
                "job": args.job,
                "events": [],
                "code": "daemon_unavailable",
                "requires_daemon": true,
            })
        },
    )
    .await
}

pub async fn doctor(args: cli::DoctorArgs, config: config::AppConfig) -> Result<()> {
    let request = node_daemon::NodeRequest::command("status")
        .to_value()
        .context("failed to encode status request")?;
    request_daemon_or_report(
        &args.endpoint,
        args.token.as_deref(),
        &config,
        request,
        args.json,
        || {
            json!({
                "ok": false,
                "kind": "daemon_doctor",
                "daemon_api": "v0.3",
                "version": env!("CARGO_PKG_VERSION"),
                "checks": [{
                    "name": "daemon_control",
                    "ok": false,
                    "blocker": "daemon_unavailable",
                    "next_action": "ssh_proxy daemon install --scope system"
                }],
                "requires_elevation": true,
            })
        },
    )
    .await
}

pub async fn vscode(args: cli::VscodeArgs, config: config::AppConfig) -> Result<()> {
    match args.command {
        cli::VscodeCommand::Up(args) => up(args.into_up_args(), config).await,
        cli::VscodeCommand::Status(args) => {
            status(
                cli::StatusArgs {
                    target: args.target,
                    workspace: args.workspace,
                    endpoint: args.endpoint,
                    token: args.token,
                    json: args.json,
                },
                config,
            )
            .await
        }
        cli::VscodeCommand::ApplySettings(args) => print_json(
            args.json,
            json!({
                "ok": true,
                "kind": "vscode_apply_settings",
                "daemon_api": "v0.3",
                "target": args.target,
                "workspace": args.workspace,
                "proxy_url": args.proxy_url,
                "job": job_status(
                    "vscode-settings:accepted",
                    "apply_remote_settings",
                    "accepted",
                    "queued",
                    "remote settings application is an allowlisted daemon job in v0.3",
                ),
            }),
        ),
        cli::VscodeCommand::Diagnose(args) => {
            doctor(
                cli::DoctorArgs {
                    endpoint: args.endpoint,
                    token: args.token,
                    json: args.json,
                },
                config,
            )
            .await
        }
    }
}

fn service_args(
    scope: cli::DaemonScope,
    json: bool,
    elevate: bool,
    no_copy: bool,
    command: cli::ServiceCommand,
) -> cli::ServiceArgs {
    cli::ServiceArgs {
        scope: scope.as_service_scope(),
        control: None,
        transport: None,
        no_transport: false,
        token: None,
        tls_transport: None,
        quic_transport: None,
        tls_cert: None,
        tls_key: None,
        tls_client_ca: None,
        report_to: Vec::new(),
        install_dir: None,
        no_copy,
        json,
        elevate,
        command,
    }
}

fn route_args_from_up(args: &cli::UpArgs) -> cli::RouteArgs {
    cli::RouteArgs {
        target: args.target.clone(),
        direction: cli::RouteDirection::RemoteUsesLocal,
        connect_mode: args.connect_mode,
        port: args.remote_port,
        bind: args.remote_bind,
        tcp_target: None,
        endpoint: args.endpoint.clone(),
        token: args.token.clone(),
        ssh_args: Vec::new(),
        user: None,
        ssh_port: None,
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
        remote_transport: cli::RemoteTransport::Auto,
        remote_tcp: None,
        remote_control: None,
        remote_quic: None,
        remote_tls: None,
        remote_ca: None,
        remote_name: "localhost".to_string(),
        remote_token: None,
        egress_proxy: Some(args.local_proxy.clone()),
        reconnect_delay_secs: None,
        reconnect_max_delay_secs: None,
        connect_timeout_secs: None,
        quic_max_bidi_streams: None,
        quic_stream_receive_window: None,
        quic_receive_window: None,
        quic_keep_alive_interval_secs: None,
        quic_idle_timeout_secs: None,
        transport_pool_size: None,
        workload_hint: Some(cli::RouteWorkloadHint::Large),
        ssh_session_pool_size: None,
        no_reconnect: false,
        local_peer: None,
        allow_plain_tcp: false,
        id: Some(
            args.id
                .clone()
                .unwrap_or_else(|| route_id_for_key(args.route_key())),
        ),
        volatile: args.volatile,
        dry_run: false,
        explain: false,
        json: args.json,
    }
}

async fn request_daemon_or_report(
    endpoint: &str,
    token: Option<&str>,
    config: &config::AppConfig,
    mut request: Value,
    compact_json: bool,
    unavailable: impl FnOnce() -> Value,
) -> Result<()> {
    let endpoint = control_socket::ControlEndpoint::parse(endpoint)?;
    if endpoint.is_tcp() {
        node_daemon::attach_auth_token(&mut request, token.or(config.daemon.token.as_deref()));
    }
    match control_socket::request(&endpoint, &format!("{request}\n")).await {
        Ok(response) => {
            print!("{response}");
            Ok(())
        }
        Err(err) => {
            let mut value = unavailable();
            if let Some(object) = value.as_object_mut() {
                object.insert("error".to_string(), json!(err.to_string()));
            }
            print_json(compact_json, value)
        }
    }
}

fn proxy_session_spec(
    target: &str,
    workspace: Option<&str>,
    local_proxy: &str,
    remote_bind: &str,
    remote_port: u16,
) -> Value {
    json!({
        "target": target,
        "workspace_id": workspace,
        "local_proxy": local_proxy,
        "remote_bind": remote_bind,
        "remote_port_policy": {
            "preferred": remote_port,
            "auto_pick": true,
        },
        "apply_policy": {
            "vscode_settings": true,
            "server_env": true,
            "git": true,
        },
    })
}

fn job_status(id: &str, kind: &str, state: &str, phase: &str, message: &str) -> Value {
    json!({
        "id": id,
        "kind": kind,
        "state": state,
        "phase": phase,
        "progress": 0,
        "blocker": Value::Null,
        "next_action": Value::Null,
        "last_error": Value::Null,
        "message": message,
    })
}

fn route_id_for_key(key: &str) -> String {
    format!("v3-{}", session_key(key))
}

fn session_key(key: &str) -> String {
    let normalized = key
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    normalized.trim_matches('-').chars().take(64).collect()
}

fn print_json(compact: bool, value: Value) -> Result<()> {
    if compact {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_ids_are_stable_and_safe() {
        assert_eq!(
            route_id_for_key("Host 126 / Workspace"),
            "v3-host-126---workspace"
        );
    }

    #[test]
    fn up_args_map_to_remote_uses_local_route() {
        let args = cli::UpArgs {
            target: "edge".to_string(),
            local_proxy: "http://127.0.0.1:10808/".to_string(),
            workspace: Some("workspace-a".to_string()),
            remote_bind: "127.0.0.1".parse().unwrap(),
            remote_port: 17890,
            connect_mode: cli::RouteConnectMode::ReverseLink,
            endpoint: control_socket::default_endpoint_string(),
            token: None,
            id: None,
            volatile: true,
            json: true,
        };
        let route = route_args_from_up(&args);
        assert_eq!(route.target, "edge");
        assert_eq!(route.direction, cli::RouteDirection::RemoteUsesLocal);
        assert_eq!(route.connect_mode, cli::RouteConnectMode::ReverseLink);
        assert_eq!(
            route.egress_proxy.as_deref(),
            Some("http://127.0.0.1:10808/")
        );
        assert_eq!(route.id.as_deref(), Some("v3-workspace-a"));
    }
}
