use std::net::SocketAddr;

use anyhow::Result;
use ssh_proxy_core::{intent::RemoteInstallIntent, model::RemotePlatform};
use ssh_proxy_lifecycle::service_provider::RemotePeerServiceSpec;

use crate::{cli, ssh_client};

pub(super) use ssh_proxy_deploy::{
    remote_clean_command, remote_doctor_command, remote_logs_command, remote_node_control_command,
    remote_node_control_json_command, remote_status_command, remote_stop_command, sh_quote,
};

pub(super) async fn default_persistent_remote_path(
    client: &ssh_client::Client,
    remote_os: cli::RemoteOs,
) -> Result<String> {
    if remote_os == cli::RemoteOs::Windows {
        return Ok(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe".to_string());
    }
    let output = client
        .exec_output(ssh_proxy_deploy::default_persistent_remote_path_command())
        .await?;
    Ok(output.trim().to_string())
}

pub(super) fn remote_resolve_peer_defaults_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    remote_os: cli::RemoteOs,
) -> String {
    let remote_platform: RemotePlatform = remote_os.into();
    ssh_proxy_deploy::remote_resolve_peer_defaults_command(
        preferred_transport,
        preferred_control,
        remote_platform,
    )
}

pub(super) fn remote_restart_command(remote_path: &str, args: &cli::InstallRemoteArgs) -> String {
    ssh_proxy_deploy::remote_restart_command(&service_spec(remote_path, args))
}

fn service_spec(remote_path: &str, args: &cli::InstallRemoteArgs) -> RemotePeerServiceSpec {
    let intent: RemoteInstallIntent = args.into();
    RemotePeerServiceSpec::from_intent(remote_path, &intent)
}
