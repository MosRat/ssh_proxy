use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{cli, config, control_socket, node_daemon, service};

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
    let spec = node_daemon::ProxySessionSpec::from_up_args(&args);
    let request = node_daemon::NodeRequest::ensure_proxy_session(spec.clone())
        .to_value()
        .context("failed to encode proxy session request")?;
    request_daemon_or_report(
        &args.endpoint,
        args.token.as_deref(),
        &config,
        request,
        args.json,
        || {
            json!({
                "ok": false,
                "kind": "proxy_session",
                "daemon_api": "v0.3",
                "spec": spec.to_value(),
                "job": job_status(
                    &spec.job_id(),
                    "ensure_proxy_session",
                    "blocked",
                    "daemon_unavailable",
                    "install or start the ssh_proxy daemon, then retry this proxy session",
                ),
                "blocker": "daemon_unavailable",
                "next_action": "ssh_proxy daemon install --scope system --elevate",
                "retry_after_ms": 1000,
                "requires_daemon": true,
                "requires_elevation": true,
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
            .map(node_daemon::ProxySessionSpec::route_id_for_key)
    }) {
        Some(id) => id,
        None => bail!("down requires --route-id, --workspace, or --target"),
    };
    let request = node_daemon::NodeRequest::proxy_session_down(Some(id.clone()), None)
        .to_value()
        .context("failed to encode proxy session down request")?;
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
                "blocker": "daemon_unavailable",
                "next_action": "ssh_proxy daemon install --scope system --elevate",
                "retry_after_ms": 1000,
                "requires_daemon": true,
                "requires_elevation": true,
            })
        },
    )
    .await
}

pub async fn status(args: cli::StatusArgs, config: config::AppConfig) -> Result<()> {
    let request = if let Some(key) = args.workspace.as_deref().or(args.target.as_deref()) {
        node_daemon::NodeRequest::proxy_session_status(
            Some(node_daemon::ProxySessionSpec::job_id_for_key(key)),
            None,
        )
    } else {
        node_daemon::NodeRequest::command("status")
    }
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
                "blocker": "daemon_unavailable",
                "requires_elevation": true,
                "next_action": "ssh_proxy daemon install --scope system --elevate",
                "retry_after_ms": 1000,
            })
        },
    )
    .await
}

pub async fn events(args: cli::EventsArgs, config: config::AppConfig) -> Result<()> {
    let request = node_daemon::NodeRequest::job_events(args.job.clone())
        .to_value()
        .context("failed to encode job events request")?;
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
                "blocker": "daemon_unavailable",
                "next_action": "ssh_proxy daemon install --scope system --elevate",
                "retry_after_ms": 1000,
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
                    "next_action": "ssh_proxy daemon install --scope system --elevate",
                    "retry_after_ms": 1000
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
    use crate::{cli, control_socket, node_daemon::ProxySessionSpec};

    #[test]
    fn up_args_map_to_proxy_session_spec() {
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
        let spec = ProxySessionSpec::from_up_args(&args);
        assert_eq!(spec.target, "edge");
        assert_eq!(spec.connect_mode, cli::RouteConnectMode::ReverseLink);
        assert_eq!(spec.local_proxy, "http://127.0.0.1:10808/");
        assert_eq!(spec.route_id(), "v3-workspace-a");
    }
}
