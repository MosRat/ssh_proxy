use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::{
    io::{self, AsyncReadExt},
    time::{self, Instant},
};

use crate::{cli, config, node_daemon, ssh_client};

mod helper;
mod remote_commands;
mod transport;

use helper::upload_helper;
use remote_commands::*;
pub(crate) use transport::{AutoTransportError, RemoteHelperTimings, TransportCandidateFailure};
pub use transport::{open_remote_helper, open_remote_reverse_socks};
#[derive(Debug, Clone)]
pub struct RemoteInstallResult {
    pub target: String,
    pub remote_node_id: Option<String>,
    pub remote_node_name: Option<String>,
    pub remote_path: String,
    pub remote_tcp: SocketAddr,
    pub remote_control: SocketAddr,
    pub remote_tls_transport: Option<SocketAddr>,
    pub remote_quic_transport: Option<SocketAddr>,
    pub remote_token: Option<String>,
}

pub async fn install_remote(mut args: cli::InstallRemoteArgs) -> Result<RemoteInstallResult> {
    let client = ssh_client::Client::connect_install_args(&args).await?;
    if args.persist != cli::PersistMode::None {
        apply_remote_auto_defaults(&client, &mut args).await?;
    }
    let remote_path = match args.remote_path.clone() {
        Some(path) => Some(path),
        None if args.persist != cli::PersistMode::None => {
            Some(default_persistent_remote_path(&client, args.remote_os).await?)
        }
        None => None,
    };
    let remote_path = upload_helper(
        &client,
        args.remote_bin.as_ref(),
        remote_path.as_deref(),
        args.remote_os,
    )
    .await?;

    match args.persist {
        cli::PersistMode::None => {
            println!("installed helper at {remote_path}");
            println!(
                "use: ssh_proxy proxy {} --remote-path {}",
                args.target, remote_path
            );
        }
        cli::PersistMode::Auto => {
            let command = remote_auto_install_command(&remote_path, &args);
            client.exec_status(command).await?;
            println!("installed persistent helper on {}", args.target);
        }
        cli::PersistMode::Systemd => {
            let command = remote_systemd_install_command(&remote_path, &args);
            client.exec_status(command).await?;
            println!("installed user systemd service on {}", args.target);
        }
        cli::PersistMode::Nohup => {
            let command = remote_nohup_start_command(&remote_path, &args, true);
            client.exec_status(command).await?;
            println!("started nohup helper on {}", args.target);
        }
        cli::PersistMode::Launchd => {
            bail!(
                "launchd persistence is intentionally explicit: upload succeeded at {remote_path}; create a LaunchAgent that runs `{remote_path} node daemon --transport 127.0.0.1:19080 --control tcp://127.0.0.1:19081`"
            );
        }
        cli::PersistMode::Schtasks => {
            bail!(
                "schtasks persistence is intentionally explicit: upload succeeded at {remote_path}; create a scheduled task that runs `{remote_path} node daemon --transport 127.0.0.1:19080 --control tcp://127.0.0.1:19081`"
            );
        }
    }
    Ok(RemoteInstallResult {
        target: args.target,
        remote_node_id: args.remote_node_id,
        remote_node_name: args.remote_node_name,
        remote_path,
        remote_tcp: args.remote_tcp,
        remote_control: args.remote_control,
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_token: args.remote_token,
    })
}

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

fn host_exec_response(
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

pub(crate) async fn refresh_remote_peer_descriptor(
    mut args: cli::InstallRemoteArgs,
) -> Result<RemoteDescriptorResult> {
    let client = ssh_client::Client::connect_install_args(&args).await?;
    let remote_path = if let Some(path) = args.remote_path.clone() {
        path
    } else {
        default_persistent_remote_path(&client, args.remote_os).await?
    };
    let command = remote_node_control_command(
        &remote_path,
        args.remote_control,
        args.remote_token.as_deref(),
        "descriptor",
    );
    let output = client.exec_output(command).await?;
    let descriptor: Value = serde_json::from_str(&output)
        .with_context(|| format!("failed to parse remote descriptor from {}", args.target))?;
    if descriptor["ok"] != true {
        bail!("remote descriptor request failed: {descriptor}");
    }
    apply_descriptor_to_install_args(&descriptor, &mut args);
    Ok(RemoteDescriptorResult {
        target: args.target,
        remote_path,
        remote_control: args.remote_control,
        remote_tcp: args.remote_tcp,
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_token: args.remote_token,
        descriptor,
    })
}

pub(crate) async fn rotate_remote_peer_token(
    mut args: cli::InstallRemoteArgs,
) -> Result<RemoteTokenRotateResult> {
    let client = ssh_client::Client::connect_install_args(&args).await?;
    let remote_path = if let Some(path) = args.remote_path.clone() {
        path
    } else {
        default_persistent_remote_path(&client, args.remote_os).await?
    };
    let command = remote_node_control_command(
        &remote_path,
        args.remote_control,
        args.remote_token.as_deref(),
        "token-rotate",
    );
    let output = client.exec_output(command).await?;
    let response: Value = serde_json::from_str(&output)
        .with_context(|| format!("failed to parse remote token rotation from {}", args.target))?;
    if response["ok"] != true {
        bail!("remote token rotation failed: {response}");
    }
    let new_token = response
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("remote token rotation did not return a token"))?
        .to_string();
    args.remote_token = Some(new_token.clone());
    let descriptor = match remote_node_control_command(
        &remote_path,
        args.remote_control,
        args.remote_token.as_deref(),
        "descriptor",
    ) {
        command => match client.exec_output(command).await {
            Ok(output) => serde_json::from_str::<Value>(&output).ok(),
            Err(_) => None,
        },
    };
    if let Some(descriptor) = &descriptor {
        apply_descriptor_to_install_args(descriptor, &mut args);
    }
    let token_metadata = response
        .get("token_metadata")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    Ok(RemoteTokenRotateResult {
        target: args.target,
        remote_path,
        remote_control: args.remote_control,
        remote_tcp: args.remote_tcp,
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_token: new_token,
        token_metadata,
        descriptor,
        response,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteDescriptorResult {
    pub(crate) target: String,
    pub(crate) remote_path: String,
    pub(crate) remote_control: SocketAddr,
    pub(crate) remote_tcp: SocketAddr,
    pub(crate) remote_tls_transport: Option<SocketAddr>,
    pub(crate) remote_quic_transport: Option<SocketAddr>,
    pub(crate) remote_token: Option<String>,
    pub(crate) descriptor: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteTokenRotateResult {
    pub(crate) target: String,
    pub(crate) remote_path: String,
    pub(crate) remote_control: SocketAddr,
    pub(crate) remote_tcp: SocketAddr,
    pub(crate) remote_tls_transport: Option<SocketAddr>,
    pub(crate) remote_quic_transport: Option<SocketAddr>,
    pub(crate) remote_token: String,
    pub(crate) token_metadata: Option<config::TokenMetadata>,
    pub(crate) descriptor: Option<Value>,
    pub(crate) response: Value,
}

pub(crate) fn record_remote_token_rotation_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteTokenRotateResult,
) -> Result<()> {
    apply_remote_token_rotation_profile(config, profile_name, result);
    config.save_default()
}

fn apply_remote_token_rotation_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteTokenRotateResult,
) {
    let profile = config.profiles.entry(profile_name.to_string()).or_default();
    if profile.target.is_none() {
        profile.target = Some(result.target.clone());
    }
    profile.remote_path = Some(result.remote_path.clone());
    profile.remote_control = Some(result.remote_control);
    profile.remote_tcp = Some(result.remote_tcp);
    profile.remote_tls = result.remote_tls_transport;
    profile.remote_quic = result.remote_quic_transport;
    profile.remote_transport = Some("auto".to_string());
    profile.remote_token = Some(result.remote_token.clone());

    let existing = config.peers.get(profile_name).cloned().unwrap_or_default();
    let descriptor = result.descriptor.as_ref();
    let node_id = descriptor
        .and_then(|descriptor| descriptor.get("node_id"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or(existing.node_id);
    let node_name = descriptor
        .and_then(|descriptor| descriptor.get("node_name"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or(existing.node_name);
    let version = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "version"))
        .or(existing.version);
    let control_api_version = descriptor
        .and_then(|descriptor| descriptor_u16_field(descriptor, "control_api_version"))
        .or(existing.control_api_version);
    let peer_protocol_version = descriptor
        .and_then(|descriptor| descriptor_u16_field(descriptor, "peer_protocol_version"))
        .or(existing.peer_protocol_version);
    let features = descriptor
        .map(|descriptor| descriptor_string_array_field(descriptor, "features"))
        .filter(|features| !features.is_empty())
        .unwrap_or(existing.features);
    let os = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "os"))
        .or(existing.os);
    let arch = descriptor
        .and_then(|descriptor| descriptor_string_field(descriptor, "arch"))
        .or(existing.arch);
    let control_endpoint = descriptor
        .and_then(|descriptor| descriptor.pointer("/endpoints/control"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or(existing.control_endpoint)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    let transport_protocols = descriptor
        .and_then(descriptor_protocols)
        .unwrap_or_else(|| {
            let mut protocols = Vec::new();
            if result.remote_quic_transport.is_some() {
                protocols.push("quic".to_string());
            }
            if result.remote_tls_transport.is_some() {
                protocols.push("tls-tcp".to_string());
            }
            protocols.push("plain-tcp".to_string());
            protocols
        });
    let token_generation = existing
        .token_metadata
        .as_ref()
        .map(|metadata| metadata.generation.saturating_add(1))
        .unwrap_or(1);
    config.record_peer(
        profile_name,
        config::PeerRecord {
            node_id,
            node_name,
            service_instance_id: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "service_instance_id"))
                .or(existing.service_instance_id),
            version,
            control_api_version,
            peer_protocol_version,
            features,
            os,
            arch,
            os_user: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "os_user"))
                .or(existing.os_user),
            data_dir: descriptor
                .and_then(|descriptor| descriptor_string_field(descriptor, "data_dir"))
                .map(Into::into)
                .or(existing.data_dir),
            target: Some(result.target.clone()),
            trust: Some("ssh-token-rotate".to_string()),
            remote_path: Some(result.remote_path.clone()),
            control_endpoint: Some(control_endpoint),
            transport: Some(result.remote_tcp),
            tls_transport: result.remote_tls_transport,
            quic_transport: result.remote_quic_transport,
            transport_protocols,
            token: Some(result.remote_token.clone()),
            token_metadata: result.token_metadata.clone().or_else(|| {
                Some(config::TokenMetadata::rotated(
                    "peer-control-transport",
                    token_generation,
                ))
            }),
            tls_server_cert_fingerprint: descriptor
                .and_then(|descriptor| descriptor.pointer("/auth/tls_server_cert_fingerprint"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or(existing.tls_server_cert_fingerprint),
            tls_client_ca_fingerprint: descriptor
                .and_then(|descriptor| descriptor.pointer("/auth/tls_client_ca_fingerprint"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or(existing.tls_client_ca_fingerprint),
            ..Default::default()
        },
    );
}

pub(crate) fn record_remote_descriptor_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteDescriptorResult,
) -> Result<()> {
    let profile = config.profiles.entry(profile_name.to_string()).or_default();
    if profile.target.is_none() {
        profile.target = Some(result.target.clone());
    }
    profile.remote_path = Some(result.remote_path.clone());
    profile.remote_control = Some(result.remote_control);
    profile.remote_tcp = Some(result.remote_tcp);
    profile.remote_tls = result.remote_tls_transport;
    profile.remote_quic = result.remote_quic_transport;
    profile.remote_transport = Some("auto".to_string());
    if let Some(token) = &result.remote_token {
        profile.remote_token = Some(token.clone());
    }
    let node_id = result
        .descriptor
        .get("node_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let node_name = result
        .descriptor
        .get("node_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let version = descriptor_string_field(&result.descriptor, "version");
    let control_api_version = descriptor_u16_field(&result.descriptor, "control_api_version");
    let peer_protocol_version = descriptor_u16_field(&result.descriptor, "peer_protocol_version");
    let features = descriptor_string_array_field(&result.descriptor, "features");
    let os = descriptor_string_field(&result.descriptor, "os");
    let arch = descriptor_string_field(&result.descriptor, "arch");
    let os_user = descriptor_string_field(&result.descriptor, "os_user");
    let data_dir = descriptor_string_field(&result.descriptor, "data_dir").map(Into::into);
    let service_instance_id = descriptor_string_field(&result.descriptor, "service_instance_id");
    let tls_server_cert_fingerprint = result
        .descriptor
        .pointer("/auth/tls_server_cert_fingerprint")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let tls_client_ca_fingerprint = result
        .descriptor
        .pointer("/auth/tls_client_ca_fingerprint")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let control_endpoint = result
        .descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("tcp://{}", result.remote_control));
    config.record_peer(
        profile_name,
        config::PeerRecord {
            node_id,
            node_name,
            service_instance_id,
            version,
            control_api_version,
            peer_protocol_version,
            features,
            os,
            arch,
            os_user,
            data_dir,
            target: Some(result.target.clone()),
            trust: Some("ssh-refresh".to_string()),
            remote_path: Some(result.remote_path.clone()),
            control_endpoint: Some(control_endpoint),
            transport: Some(result.remote_tcp),
            tls_transport: result.remote_tls_transport,
            quic_transport: result.remote_quic_transport,
            transport_protocols: descriptor_protocols(&result.descriptor)
                .unwrap_or_else(|| remote_descriptor_protocols(result)),
            token: result.remote_token.clone(),
            token_metadata: result
                .descriptor
                .pointer("/auth/token_metadata")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .or_else(|| {
                    result
                        .remote_token
                        .as_ref()
                        .map(|_| config::TokenMetadata::new("peer-control-transport"))
                }),
            tls_server_cert_fingerprint,
            tls_client_ca_fingerprint,
            ..Default::default()
        },
    );
    config.save_default()
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

fn apply_descriptor_to_install_args(descriptor: &Value, args: &mut cli::InstallRemoteArgs) {
    if let Some(control) = descriptor
        .pointer("/endpoints/control")
        .and_then(Value::as_str)
        .and_then(parse_tcp_endpoint)
    {
        args.remote_control = control;
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_addr)
    {
        args.remote_tcp = transport;
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/tls_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_addr)
    {
        args.remote_tls_transport = Some(transport);
    }
    if let Some(transport) = descriptor
        .pointer("/endpoints/quic_transport")
        .and_then(Value::as_str)
        .and_then(parse_socket_addr)
    {
        args.remote_quic_transport = Some(transport);
    }
}

fn parse_tcp_endpoint(value: &str) -> Option<SocketAddr> {
    value.strip_prefix("tcp://").unwrap_or(value).parse().ok()
}

fn parse_socket_addr(value: &str) -> Option<SocketAddr> {
    value.parse().ok()
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

fn descriptor_string_field(descriptor: &Value, field: &str) -> Option<String> {
    descriptor
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn descriptor_u16_field(descriptor: &Value, field: &str) -> Option<u16> {
    descriptor
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
}

fn descriptor_string_array_field(descriptor: &Value, field: &str) -> Vec<String> {
    descriptor
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

fn remote_descriptor_protocols(result: &RemoteDescriptorResult) -> Vec<String> {
    let mut protocols = Vec::new();
    if result.remote_quic_transport.is_some() {
        protocols.push("quic".to_string());
    }
    if result.remote_tls_transport.is_some() {
        protocols.push("tls-tcp".to_string());
    }
    protocols.push("plain-tcp".to_string());
    protocols
}

pub(crate) fn record_remote_install_profile(
    config: &mut config::AppConfig,
    profile_name: &str,
    result: &RemoteInstallResult,
) -> Result<()> {
    let profile = config.profiles.entry(profile_name.to_string()).or_default();
    if profile.target.is_none() {
        profile.target = Some(result.target.clone());
    }
    profile.remote_path = Some(result.remote_path.clone());
    profile.remote_tcp = Some(result.remote_tcp);
    profile.remote_control = Some(result.remote_control);
    profile.remote_tls = result.remote_tls_transport;
    profile.remote_quic = result.remote_quic_transport;
    profile.remote_transport = Some("auto".to_string());
    if let Some(token) = &result.remote_token {
        profile.remote_token = Some(token.clone());
    }
    config.record_peer(
        profile_name,
        config::PeerRecord {
            node_id: result.remote_node_id.clone(),
            node_name: result.remote_node_name.clone(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            control_api_version: Some(node_daemon::control_api_version()),
            peer_protocol_version: Some(node_daemon::peer_protocol_version()),
            features: node_daemon::peer_protocol_features(),
            os: None,
            arch: None,
            target: Some(result.target.clone()),
            trust: Some("ssh-bootstrap".to_string()),
            remote_path: Some(result.remote_path.clone()),
            control_endpoint: Some(format!("tcp://{}", result.remote_control)),
            transport: Some(result.remote_tcp),
            tls_transport: result.remote_tls_transport,
            quic_transport: result.remote_quic_transport,
            transport_protocols: remote_transport_protocols(result),
            token: result.remote_token.clone(),
            token_metadata: result
                .remote_token
                .as_ref()
                .map(|_| config::TokenMetadata::new("peer-control-transport")),
            ..Default::default()
        },
    );
    config.save_default()
}

fn remote_transport_protocols(result: &RemoteInstallResult) -> Vec<String> {
    let mut protocols = Vec::new();
    if result.remote_quic_transport.is_some() {
        protocols.push("quic".to_string());
    }
    if result.remote_tls_transport.is_some() {
        protocols.push("tls-tcp".to_string());
    }
    protocols.push("plain-tcp".to_string());
    protocols
}

async fn apply_remote_auto_defaults(
    client: &ssh_client::Client,
    args: &mut cli::InstallRemoteArgs,
) -> Result<()> {
    if args.remote_token.is_none() {
        args.remote_token = Some(config::generate_token()?);
    }
    let token = args.remote_token.as_deref().unwrap_or_default();
    let output = client
        .exec_output(remote_write_config_command(
            args.remote_tcp,
            args.remote_control,
            token,
            args.local_node_id.as_deref(),
            args.local_node_name.as_deref(),
            args.local_control_endpoint.as_deref(),
            args.local_transport,
        ))
        .await?;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("transport=") {
            args.remote_tcp = value
                .parse()
                .with_context(|| format!("invalid remote transport address {value:?}"))?;
        } else if let Some(value) = line.strip_prefix("control=") {
            args.remote_control = value
                .parse()
                .with_context(|| format!("invalid remote control address {value:?}"))?;
        } else if let Some(value) = line.strip_prefix("node_id=") {
            args.remote_node_id = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("node_name=") {
            args.remote_node_name = Some(value.to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_updates_install_endpoints() {
        let descriptor = serde_json::json!({
            "endpoints": {
                "control": "tcp://127.0.0.1:29181",
                "transport": "127.0.0.1:29180",
                "tls_transport": "127.0.0.1:29182",
                "quic_transport": "127.0.0.1:29183"
            },
            "transport_protocols": ["quic", "tls-tcp", "plain-tcp"]
        });
        let mut args = cli::InstallRemoteArgs {
            target: "peer".to_string(),
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
            remote_os: cli::RemoteOs::Auto,
            remote_token: None,
            remote_tcp: "127.0.0.1:19080".parse().unwrap(),
            remote_control: "127.0.0.1:19081".parse().unwrap(),
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

        apply_descriptor_to_install_args(&descriptor, &mut args);

        assert_eq!(args.remote_control, "127.0.0.1:29181".parse().unwrap());
        assert_eq!(args.remote_tcp, "127.0.0.1:29180".parse().unwrap());
        assert_eq!(
            args.remote_tls_transport,
            Some("127.0.0.1:29182".parse().unwrap())
        );
        assert_eq!(
            args.remote_quic_transport,
            Some("127.0.0.1:29183".parse().unwrap())
        );
        assert_eq!(
            descriptor_protocols(&descriptor).unwrap(),
            vec!["quic", "tls-tcp", "plain-tcp"]
        );
    }

    #[test]
    fn token_rotation_updates_profile_and_peer_record() {
        let mut config = config::AppConfig::default();
        config.peers.insert(
            "peer".to_string(),
            config::PeerRecord {
                node_id: Some("old-node".to_string()),
                node_name: Some("old-name".to_string()),
                target: Some("old-target".to_string()),
                remote_path: Some("/old/bin/ssh_proxy".to_string()),
                control_endpoint: Some("tcp://127.0.0.1:19081".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                token: Some("old-token".to_string()),
                ..Default::default()
            },
        );
        let result = RemoteTokenRotateResult {
            target: "host".to_string(),
            remote_path: "/home/me/bin/ssh_proxy".to_string(),
            remote_control: "127.0.0.1:29181".parse().unwrap(),
            remote_tcp: "127.0.0.1:29180".parse().unwrap(),
            remote_tls_transport: Some("127.0.0.1:29182".parse().unwrap()),
            remote_quic_transport: None,
            remote_token: "new-token".to_string(),
            token_metadata: Some(config::TokenMetadata::rotated("peer-control-transport", 2)),
            descriptor: Some(serde_json::json!({
                "node_id": "new-node",
                "node_name": "new-name",
                "version": "0.3.0",
                "control_api_version": 1,
                "peer_protocol_version": 1,
                "features": ["frames-v1", "token-auth-v1"],
                "os": "linux",
                "arch": "x86_64",
                "endpoints": {
                    "control": "tcp://127.0.0.1:29181",
                    "transport": "127.0.0.1:29180",
                    "tls_transport": "127.0.0.1:29182"
                },
                "transport_protocols": ["tls-tcp", "plain-tcp"]
            })),
            response: serde_json::json!({"ok": true}),
        };

        apply_remote_token_rotation_profile(&mut config, "peer", &result);

        let profile = config.profiles.get("peer").unwrap();
        assert_eq!(profile.remote_token.as_deref(), Some("new-token"));
        assert_eq!(profile.remote_tls, Some("127.0.0.1:29182".parse().unwrap()));
        let peer = config.peers.get("peer").unwrap();
        assert_eq!(peer.node_id.as_deref(), Some("new-node"));
        assert_eq!(peer.node_name.as_deref(), Some("new-name"));
        assert_eq!(peer.version.as_deref(), Some("0.3.0"));
        assert_eq!(peer.control_api_version, Some(1));
        assert_eq!(peer.peer_protocol_version, Some(1));
        assert_eq!(peer.features, vec!["frames-v1", "token-auth-v1"]);
        assert_eq!(peer.os.as_deref(), Some("linux"));
        assert_eq!(peer.arch.as_deref(), Some("x86_64"));
        assert_eq!(peer.trust.as_deref(), Some("ssh-token-rotate"));
        assert_eq!(peer.token.as_deref(), Some("new-token"));
        assert_eq!(peer.transport_protocols, vec!["tls-tcp", "plain-tcp"]);
        assert_eq!(
            peer.token_metadata.as_ref().unwrap().scope,
            "peer-control-transport"
        );
    }

    #[test]
    fn host_exec_response_has_stable_json_contract() {
        let value = host_exec_response(
            "edge",
            "remote setup",
            Some(7),
            "hello\n",
            "warning\n",
            42,
            false,
        );

        assert_eq!(value["ok"], false);
        assert_eq!(value["kind"], "host_exec");
        assert_eq!(value["target"], "edge");
        assert_eq!(value["label"], "remote setup");
        assert_eq!(value["exit_code"], 7);
        assert_eq!(value["stdout"], "hello\n");
        assert_eq!(value["stderr"], "warning\n");
        assert_eq!(value["duration_ms"], 42);
        assert_eq!(value["timed_out"], false);
    }

    #[test]
    fn host_exec_timeout_response_uses_null_exit_code() {
        let value = host_exec_response(
            "edge",
            "remote setup",
            None,
            "",
            "host exec timed out after 3s",
            3001,
            true,
        );

        assert_eq!(value["ok"], false);
        assert!(value["exit_code"].is_null());
        assert_eq!(value["stderr"], "host exec timed out after 3s");
        assert_eq!(value["timed_out"], true);
    }
}
