use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::{
    cli, config, control_socket, diagnostics, install_report, node_daemon, repair, service,
};

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
        cli::DaemonCommand::Update { source } => {
            let update_source = daemon_update_source(source.as_deref())?;
            let request = node_daemon::NodeRequest::daemon_update(update_source.clone())
                .to_value()
                .context("failed to encode daemon update request")?;
            request_daemon_or_report(
                &control_socket::default_endpoint_string(),
                None,
                &config,
                request,
                args.json,
                || {
                    json!({
                        "ok": false,
                        "kind": "daemon_update",
                        "daemon_api": "v0.3",
                        "job": job_status(
                            "self-update:pending",
                            "self_update",
                            "blocked",
                            "daemon_unavailable",
                            "daemon self-update requires the running daemon",
                        ),
                        "source": update_source.clone(),
                        "requires_daemon": true,
                        "requires_elevation": true,
                        "blocker": "daemon_unavailable",
                        "next_action": "ssh_proxy daemon install --scope system --elevate",
                    })
                },
            )
            .await
        }
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

pub async fn daemon_install_worker(
    args: cli::DaemonInstallWorkerArgs,
    config: config::AppConfig,
) -> Result<()> {
    install_worker(
        args.scope,
        args.json,
        args.no_copy,
        args.install_log,
        config,
    )
    .await
}

async fn install_worker(
    scope: cli::DaemonScope,
    json_output: bool,
    no_copy: bool,
    log: PathBuf,
    config: config::AppConfig,
) -> Result<()> {
    let install_id = format!("install-{}-{}", std::process::id(), now_unix());
    install_report::append_install_event(
        &log,
        &install_id,
        "running",
        "prepare",
        "preparing elevated daemon install",
        None,
    )?;
    let install_result = service::run(
        service_args(scope, false, false, no_copy, cli::ServiceCommand::Install),
        config,
    )
    .await;
    if let Err(err) = install_result {
        install_report::append_install_event(
            &log,
            &install_id,
            "failed",
            "install_service",
            &err.to_string(),
            Some("requires_elevation"),
        )?;
        return Err(err);
    }
    install_report::append_install_event(
        &log,
        &install_id,
        "running",
        "health_check",
        "waiting for daemon control endpoint health",
        None,
    )?;
    match wait_for_daemon_health().await {
        Ok(()) => {
            install_report::append_install_event(
                &log,
                &install_id,
                "healthy",
                "healthy",
                "daemon installed and control endpoint is healthy",
                None,
            )?;
            if json_output {
                print_json(false, install_report::install_report_from_log(&log))?;
            }
            Ok(())
        }
        Err(err) => {
            install_report::append_install_event(
                &log,
                &install_id,
                "failed",
                "health_check",
                &err.to_string(),
                Some("daemon_unavailable"),
            )?;
            Err(err)
        }
    }
}

async fn wait_for_daemon_health() -> Result<()> {
    let endpoint =
        control_socket::ControlEndpoint::parse(&control_socket::default_endpoint_string())?;
    let request = node_daemon::NodeRequest::command("status")
        .to_value()
        .context("failed to encode daemon status request")?;
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(20);
    loop {
        match control_socket::request(&endpoint, &format!("{request}\n")).await {
            Ok(response) => {
                let value = serde_json::from_str::<Value>(&response).unwrap_or(Value::Null);
                if value.get("ok").and_then(Value::as_bool) == Some(true) {
                    return Ok(());
                }
            }
            Err(_) if tokio::time::Instant::now() < deadline => {}
            Err(err) => return Err(err).context("daemon health check failed after install"),
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("daemon health check timed out after install");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
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
    let endpoint = control_socket::ControlEndpoint::parse(&args.endpoint)?;
    let mut request = request;
    node_daemon::attach_auth_token(
        &mut request,
        args.token.as_deref().or(config.daemon.token.as_deref()),
    );
    match control_socket::request(&endpoint, &format!("{request}\n")).await {
        Ok(response) if args.report => {
            let status = serde_json::from_str::<Value>(&response).unwrap_or_else(|_| {
                json!({
                    "ok": false,
                    "kind": "daemon_status",
                    "error": "failed to parse daemon status response"
                })
            });
            let mut report = diagnostics::doctor_report(&config, Some(status.clone()));
            if let Some(target) = &args.target {
                report["peer_report"] = diagnostics::peer_report(&config, Some(&status), target);
            }
            print_json(
                args.json,
                json!({
                    "ok": status.get("ok").and_then(Value::as_bool).unwrap_or(false),
                    "kind": "daemon_doctor",
                    "daemon_api": "v0.3",
                    "target": args.target,
                    "status": diagnostics::redact_value(&status),
                    "dependencies": report.get("dependencies").cloned().unwrap_or_else(|| json!([])),
                    "peer_report": report.get("peer_report").cloned().unwrap_or(Value::Null),
                    "report": report,
                }),
            )
        }
        Ok(response) => {
            if let Some(value) = normalize_daemon_response(&response) {
                print_json(args.json, value)
            } else {
                print!("{response}");
                Ok(())
            }
        }
        Err(err) => {
            let mut value = daemon_unavailable_doctor(&config, args.report, args.target.as_deref());
            annotate_control_error(&mut value, &err);
            attach_top_level_repair_action(&mut value);
            print_json(args.json, value)
        }
    }
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
        cli::VscodeCommand::ApplySettings(args) => {
            let request = node_daemon::NodeRequest::apply_remote_settings(
                args.target.clone(),
                args.workspace.clone(),
                args.proxy_url.clone(),
            )
            .to_value()
            .context("failed to encode remote settings request")?;
            request_daemon_or_report(
                &args.endpoint,
                args.token.as_deref(),
                &config,
                request,
                args.json,
                || {
                    json!({
                        "ok": false,
                        "kind": "vscode_apply_settings",
                        "daemon_api": "v0.3",
                        "target": args.target,
                        "workspace": args.workspace,
                        "proxy_url": args.proxy_url,
                        "job": job_status(
                            "vscode-settings:blocked",
                            "apply_remote_settings",
                            "blocked",
                            "daemon_unavailable",
                            "remote settings application requires the running daemon",
                        ),
                        "blocker": "daemon_unavailable",
                        "next_action": "ssh_proxy daemon install --scope system --elevate",
                        "requires_daemon": true,
                        "retry_after_ms": 1000,
                    })
                },
            )
            .await
        }
        cli::VscodeCommand::Diagnose(args) => {
            doctor(
                cli::DoctorArgs {
                    target: None,
                    endpoint: args.endpoint,
                    token: args.token,
                    json: args.json,
                    report: args.report,
                },
                config,
            )
            .await
        }
    }
}

fn daemon_unavailable_doctor(
    config: &config::AppConfig,
    include_report: bool,
    target: Option<&str>,
) -> Value {
    let mut value = json!({
        "ok": false,
        "kind": "daemon_doctor",
        "daemon_api": "v0.3",
        "version": env!("CARGO_PKG_VERSION"),
        "target": target,
        "checks": [{
            "name": "daemon_control",
            "ok": false,
            "blocker": "daemon_unavailable",
            "next_action": "ssh_proxy daemon install --scope system --elevate",
            "repair_action": repair::action_value_for_blocker("daemon_unavailable"),
            "retry_after_ms": 1000
        }],
        "dependencies": diagnostics::daemon_dependency_report(config, None),
        "blocker": "daemon_unavailable",
        "next_action": "ssh_proxy daemon install --scope system --elevate",
        "repair_action": repair::action_value_for_blocker("daemon_unavailable"),
        "requires_elevation": true,
    });
    if include_report {
        let mut report = diagnostics::doctor_report(config, None);
        if let Some(target) = target {
            report["peer_report"] = diagnostics::peer_report(config, None, target);
            value["peer_report"] = report["peer_report"].clone();
        }
        value["report"] = report;
    }
    value
}

fn service_args(
    scope: cli::DaemonScope,
    json: bool,
    elevate: bool,
    no_copy: bool,
    command: cli::ServiceCommand,
) -> cli::ServiceArgs {
    let control = match command {
        cli::ServiceCommand::Ensure | cli::ServiceCommand::Install | cli::ServiceCommand::Print => {
            Some(control_socket::default_endpoint_string())
        }
        cli::ServiceCommand::Uninstall
        | cli::ServiceCommand::Start
        | cli::ServiceCommand::Stop
        | cli::ServiceCommand::Status => None,
    };
    cli::ServiceArgs {
        scope: scope.as_service_scope(),
        control,
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
    node_daemon::attach_auth_token(&mut request, token.or(config.daemon.token.as_deref()));
    match control_socket::request(&endpoint, &format!("{request}\n")).await {
        Ok(response) => {
            if let Some(value) = normalize_daemon_response(&response) {
                print_json(compact_json, value)
            } else {
                print!("{response}");
                Ok(())
            }
        }
        Err(err) => {
            let mut value = unavailable();
            annotate_control_error(&mut value, &err);
            attach_top_level_repair_action(&mut value);
            print_json(compact_json, value)
        }
    }
}

fn normalize_daemon_response(response: &str) -> Option<Value> {
    let mut value = serde_json::from_str::<Value>(response).ok()?;
    let code = value.get("code").and_then(Value::as_str);
    let error = value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let blocker = match (code, error) {
        (Some("unauthorized"), error) if error.contains("node control token is required") => {
            "node_control_token_required"
        }
        (Some("unauthorized"), error) if error.contains("invalid node control token") => {
            "invalid_node_control_token"
        }
        _ => return None,
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("code".to_string(), json!(blocker));
        object.insert("blocker".to_string(), json!(blocker));
        object.insert("requires_elevation".to_string(), json!(true));
        object.insert(
            "message".to_string(),
            json!("ssh_proxy daemon configuration is stale and needs repair"),
        );
        object.insert(
            "next_action".to_string(),
            json!("ssh_proxy daemon install --scope system --elevate"),
        );
        repair::attach_repair_action(object, blocker);
    }
    Some(value)
}

fn annotate_control_error(value: &mut Value, err: &anyhow::Error) {
    let access_denied = err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(is_access_denied)
    }) || err.to_string().contains("Access is denied");
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("error".to_string(), json!(err.to_string()));
    if !access_denied {
        return;
    }
    object.insert("code".to_string(), json!("daemon_pipe_access_denied"));
    object.insert("blocker".to_string(), json!("daemon_pipe_access_denied"));
    object.insert(
        "message".to_string(),
        json!("ssh_proxy daemon pipe denied this user"),
    );
    object.insert(
        "next_action".to_string(),
        json!("ssh_proxy daemon install --scope system --elevate"),
    );
    object.insert("requires_elevation".to_string(), json!(true));
    object.insert("retry_after_ms".to_string(), json!(1000));
    repair::attach_repair_action(object, "daemon_pipe_access_denied");
    if let Some(job) = object.get_mut("job").and_then(Value::as_object_mut) {
        job.insert("state".to_string(), json!("blocked"));
        job.insert("phase".to_string(), json!("daemon_pipe_access_denied"));
        job.insert("blocker".to_string(), json!("daemon_pipe_access_denied"));
        repair::attach_repair_action(job, "daemon_pipe_access_denied");
        job.insert(
            "message".to_string(),
            json!("daemon pipe denied this user; reinstall or restart daemon to repair pipe ACL"),
        );
    }
}

fn attach_top_level_repair_action(value: &mut Value) {
    let blocker = value
        .get("blocker")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let (Some(blocker), Some(object)) = (blocker, value.as_object_mut()) {
        repair::attach_repair_action(object, &blocker);
    }
    if let Some(job) = value.get_mut("job").and_then(Value::as_object_mut) {
        let job_blocker = job
            .get("blocker")
            .and_then(Value::as_str)
            .or_else(|| job.get("phase").and_then(Value::as_str))
            .map(str::to_string);
        if let Some(job_blocker) = job_blocker {
            repair::attach_repair_action(job, &job_blocker);
        }
    }
}

fn is_access_denied(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied || error.raw_os_error() == Some(5)
}

fn job_status(id: &str, kind: &str, state: &str, phase: &str, message: &str) -> Value {
    let blocker = (state == "blocked").then_some(phase);
    json!({
        "id": id,
        "kind": kind,
        "state": state,
        "phase": phase,
        "progress": 0,
        "blocker": blocker,
        "next_action": Value::Null,
        "repair_action": blocker.map(repair::action_value_for_blocker).unwrap_or(Value::Null),
        "last_error": Value::Null,
        "message": message,
    })
}

fn daemon_update_source(source: Option<&Path>) -> Result<Option<String>> {
    source
        .map(|path| {
            path.canonicalize()
                .with_context(|| format!("failed to resolve update source {}", path.display()))
                .map(|path| path.display().to_string())
        })
        .transpose()
}

fn print_json(compact: bool, value: Value) -> Result<()> {
    if compact {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
            ssh_host_name: None,
            ssh_user: None,
            ssh_port: None,
            ssh_identity: Vec::new(),
            ssh_config: None,
            ssh_known_hosts: None,
            ssh_jump: Vec::new(),
            ssh_accept_new: false,
            workspace_paths: Vec::new(),
            server_dir: ".vscode-server".to_string(),
            no_proxy: "localhost,127.0.0.1,::1".to_string(),
            proxy_support: "override".to_string(),
            no_remote_machine_settings: false,
            no_terminal_env: false,
            no_server_env: false,
            no_git: false,
            no_git_global: false,
            no_git_workspace: false,
            no_git_force_override: false,
            no_remote_status_file: false,
            no_verify_remote_port: false,
            volatile: true,
            json: true,
        };
        let spec = ProxySessionSpec::from_up_args(&args);
        assert_eq!(spec.target, "edge");
        assert_eq!(spec.connect_mode, cli::RouteConnectMode::ReverseLink);
        assert_eq!(spec.local_proxy, "http://127.0.0.1:10808/");
        assert_eq!(spec.route_id(), "v3-workspace-a");
    }

    #[test]
    fn daemon_install_service_args_use_production_control_endpoint() {
        let args = super::service_args(
            cli::DaemonScope::System,
            true,
            true,
            false,
            cli::ServiceCommand::Install,
        );
        assert_eq!(
            args.control,
            Some(control_socket::default_endpoint_string())
        );
        assert!(args.json);
        assert!(args.elevate);
    }

    #[test]
    fn daemon_start_service_args_do_not_rewrite_control_endpoint() {
        let args = super::service_args(
            cli::DaemonScope::System,
            false,
            false,
            false,
            cli::ServiceCommand::Start,
        );
        assert_eq!(args.control, None);
    }

    #[test]
    fn daemon_update_source_is_canonicalized_for_system_daemon() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-update-source-{}.bin",
            std::process::id()
        ));
        std::fs::write(&path, b"candidate").expect("write update source");

        let source = super::daemon_update_source(Some(&path)).expect("canonicalize update source");
        let source = PathBuf::from(source.expect("source path"));

        assert!(source.is_absolute());
        assert_eq!(source.file_name(), path.file_name());

        let _ = std::fs::remove_file(path);
    }
}
