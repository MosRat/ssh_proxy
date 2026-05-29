use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::{
    io::{self, AsyncReadExt},
    time::{self, Instant},
};

use crate::{cli, config, node_daemon, ssh_client};

use super::{
    install_remote, record_remote_install_profile,
    remote_commands::{
        default_persistent_remote_path, remote_clean_command, remote_doctor_command,
        remote_logs_command, remote_node_control_command, remote_node_control_json_command,
        remote_restart_command, remote_status_command, remote_stop_command, sh_quote,
    },
};

pub async fn host(mut args: cli::HostArgs, mut config: config::AppConfig) -> Result<()> {
    let profile_name = args.target.clone();
    let mut install_args = install_args_from_host(&args);
    config.apply_install_defaults(&mut install_args, Some(&profile_name))?;

    match args.command.clone() {
        cli::HostCommand::Start => {
            let local_identity = config.ensure_node_identity()?;
            if args.remote_path.is_some() {
                install_args.remote_path = args.remote_path.clone();
            }
            if args.remote_bin.is_some() {
                install_args.remote_bin = args.remote_bin.clone();
            }
            install_args.remote_os = args.remote_os;
            install_args.remote_token = args.remote_token.clone().or(install_args.remote_token);
            if install_args.remote_token.is_none() {
                install_args.remote_token = Some(config::generate_token()?);
            }
            install_args.remote_tcp = args.remote_tcp;
            install_args.remote_control = args.remote_control;
            install_args.remote_tls_transport = args.remote_tls_transport;
            install_args.remote_quic_transport = args.remote_quic_transport;
            install_args.remote_tls_cert = args.remote_tls_cert.clone();
            install_args.remote_tls_key = args.remote_tls_key.clone();
            install_args.remote_tls_client_ca = args.remote_tls_client_ca.clone();
            install_args.local_node_id = local_identity.node_id.clone();
            install_args.local_node_name = local_identity.node_name.clone();
            install_args.local_control_endpoint = config.daemon.control_endpoint.clone();
            install_args.local_transport = config.daemon.transport_listen;
            install_args.persist = args.persist;
            let result = install_remote(install_args).await?;
            record_remote_install_profile(&mut config, &profile_name, &result)?;
        }
        cli::HostCommand::Exec(exec) => {
            apply_resolved_target_to_host(&mut args, &install_args);
            let client = ssh_client::Client::connect_install_args(&install_args).await?;
            run_host_exec(&args.target, exec, &client).await?;
        }
        cli::HostCommand::Status
        | cli::HostCommand::NodeStatus
        | cli::HostCommand::NodeDescriptor
        | cli::HostCommand::NodeLinks
        | cli::HostCommand::NodeForward(_)
        | cli::HostCommand::NodeReverse(_)
        | cli::HostCommand::NodeStopRoute { .. }
        | cli::HostCommand::NodeRestartRoute { .. }
        | cli::HostCommand::NodeRoutes
        | cli::HostCommand::NodeConnect { .. }
        | cli::HostCommand::NodeDisconnect { .. }
        | cli::HostCommand::Stop
        | cli::HostCommand::Restart
        | cli::HostCommand::Logs { .. }
        | cli::HostCommand::Clean
        | cli::HostCommand::Doctor => {
            apply_resolved_target_to_host(&mut args, &install_args);
            let client = ssh_client::Client::connect_install_args(&install_args).await?;
            let remote_path = if let Some(path) = args
                .remote_path
                .clone()
                .or(install_args.remote_path.clone())
            {
                path
            } else {
                default_persistent_remote_path(&client, args.remote_os).await?
            };
            let command = match args.command {
                cli::HostCommand::Status => remote_status_command(&remote_path, args.remote_tcp),
                cli::HostCommand::NodeStatus => remote_node_control_command(
                    &remote_path,
                    args.remote_control,
                    args.remote_token.as_deref(),
                    "status",
                ),
                cli::HostCommand::NodeDescriptor => remote_node_control_command(
                    &remote_path,
                    args.remote_control,
                    args.remote_token.as_deref(),
                    "descriptor",
                ),
                cli::HostCommand::NodeLinks => remote_node_control_command(
                    &remote_path,
                    args.remote_control,
                    args.remote_token.as_deref(),
                    "links",
                ),
                cli::HostCommand::NodeForward(route) => {
                    let id = route
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("forward:{}->{}", route.listen, route.target));
                    let persist = !route.volatile;
                    let proxy = node_daemon::proxy_args_from_node_forward(route);
                    let request = node_daemon::NodeRequest::route_start_forward(id, persist, proxy)
                        .to_value()?;
                    remote_node_control_json_command(
                        &remote_path,
                        args.remote_control,
                        args.remote_token.as_deref(),
                        &request,
                    )
                }
                cli::HostCommand::NodeReverse(route) => {
                    let id = route.id.clone().unwrap_or_else(|| {
                        format!("reverse:{}<-{}", route.remote_listen, route.target)
                    });
                    let persist = !route.volatile;
                    let reverse = node_daemon::reverse_args_from_node_reverse(route);
                    let request =
                        node_daemon::NodeRequest::route_start_reverse(id, persist, reverse, None)
                            .to_value()?;
                    remote_node_control_json_command(
                        &remote_path,
                        args.remote_control,
                        args.remote_token.as_deref(),
                        &request,
                    )
                }
                cli::HostCommand::NodeStopRoute { id } => {
                    let request = node_daemon::NodeRequest::route_stop(id).to_value()?;
                    remote_node_control_json_command(
                        &remote_path,
                        args.remote_control,
                        args.remote_token.as_deref(),
                        &request,
                    )
                }
                cli::HostCommand::NodeRestartRoute { id } => {
                    let request = node_daemon::NodeRequest::route_restart(id).to_value()?;
                    remote_node_control_json_command(
                        &remote_path,
                        args.remote_control,
                        args.remote_token.as_deref(),
                        &request,
                    )
                }
                cli::HostCommand::NodeRoutes => remote_node_control_command(
                    &remote_path,
                    args.remote_control,
                    args.remote_token.as_deref(),
                    "routes",
                ),
                cli::HostCommand::NodeConnect { profile } => remote_node_control_command(
                    &remote_path,
                    args.remote_control,
                    args.remote_token.as_deref(),
                    &format!("connect {}", sh_quote(&profile)),
                ),
                cli::HostCommand::NodeDisconnect { profile } => remote_node_control_command(
                    &remote_path,
                    args.remote_control,
                    args.remote_token.as_deref(),
                    &format!("disconnect {}", sh_quote(&profile)),
                ),
                cli::HostCommand::Exec(_) => unreachable!(),
                cli::HostCommand::Stop => remote_stop_command(args.remote_tcp),
                cli::HostCommand::Restart => remote_restart_command(&remote_path, &install_args),
                cli::HostCommand::Logs { lines } => remote_logs_command(args.remote_tcp, lines),
                cli::HostCommand::Clean => remote_clean_command(&remote_path, args.remote_tcp),
                cli::HostCommand::Doctor => remote_doctor_command(&remote_path, args.remote_tcp),
                cli::HostCommand::Start => unreachable!(),
            };
            let output = client.exec_output(command).await?;
            print!("{output}");
        }
    }
    Ok(())
}

async fn run_host_exec(
    target: &str,
    exec: cli::HostExecArgs,
    client: &ssh_client::Client,
) -> Result<()> {
    if !exec.stdin {
        bail!("host exec currently requires --stdin");
    }

    let mut script = Vec::new();
    io::stdin()
        .read_to_end(&mut script)
        .await
        .context("failed to read host exec stdin")?;

    let started = Instant::now();
    let timeout = Duration::from_secs(exec.timeout_secs.max(1));
    let result = time::timeout(
        timeout,
        client.exec_capture("sh -s".to_string(), Some(script)),
    )
    .await;
    let duration_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(output)) => {
            if exec.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&host_exec_response(
                        target,
                        &exec.label,
                        Some(output.exit_status),
                        &output.stdout,
                        &output.stderr,
                        duration_ms,
                        false,
                    ))?
                );
                return Ok(());
            }
            print!("{}", output.stdout);
            if output.exit_status != 0 {
                bail!(
                    "host exec {:?} exited with status {}: {}",
                    exec.label,
                    output.exit_status,
                    output.stderr.trim()
                );
            }
        }
        Ok(Err(err)) => return Err(err).context("host exec failed"),
        Err(_) if exec.json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&host_exec_response(
                    target,
                    &exec.label,
                    None,
                    "",
                    &format!("host exec timed out after {}s", exec.timeout_secs.max(1)),
                    duration_ms,
                    true,
                ))?
            );
            return Ok(());
        }
        Err(_) => bail!(
            "host exec {:?} timed out after {}s",
            exec.label,
            exec.timeout_secs.max(1)
        ),
    }
    Ok(())
}

pub(super) fn host_exec_response(
    target: &str,
    label: &str,
    exit_code: Option<u32>,
    stdout: &str,
    stderr: &str,
    duration_ms: u64,
    timed_out: bool,
) -> Value {
    serde_json::json!({
        "ok": exit_code == Some(0) && !timed_out,
        "kind": "host_exec",
        "target": target,
        "label": label,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "duration_ms": duration_ms,
        "timed_out": timed_out,
    })
}

fn install_args_from_host(args: &cli::HostArgs) -> cli::InstallRemoteArgs {
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
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_tls_cert: args.remote_tls_cert.clone(),
        remote_tls_key: args.remote_tls_key.clone(),
        remote_tls_client_ca: args.remote_tls_client_ca.clone(),
        persist: cli::PersistMode::None,
    }
}

fn apply_resolved_target_to_host(args: &mut cli::HostArgs, install: &cli::InstallRemoteArgs) {
    args.target = install.target.clone();
    if args.ssh_args.is_empty() {
        args.ssh_args = install.ssh_args.clone();
    }
    args.user = args.user.take().or_else(|| install.user.clone());
    args.port = args.port.or(install.port);
    if args.identity.is_empty() {
        args.identity = install.identity.clone();
    }
    args.config = args.config.take().or_else(|| install.config.clone());
    args.known_hosts = args
        .known_hosts
        .take()
        .or_else(|| install.known_hosts.clone());
    args.accept_new |= install.accept_new;
    if args.jump.is_empty() {
        args.jump = install.jump.clone();
    }
    args.remote_path = args
        .remote_path
        .take()
        .or_else(|| install.remote_path.clone());
    args.remote_bin = args
        .remote_bin
        .take()
        .or_else(|| install.remote_bin.clone());
    args.remote_token = args
        .remote_token
        .take()
        .or_else(|| install.remote_token.clone());
    args.remote_tcp = install.remote_tcp;
    args.remote_control = install.remote_control;
}
