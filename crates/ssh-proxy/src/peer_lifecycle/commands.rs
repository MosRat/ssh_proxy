use crate::cli;
use ssh_proxy_core::{intent::RemoteInstallIntent, model::RemotePlatform};
use ssh_proxy_lifecycle::service_provider::RemotePeerServiceSpec;

use super::artifacts::PeerArtifact;

pub(crate) fn remote_write_peer_artifact_command(
    artifact: PeerArtifact,
    remote_os: cli::RemoteOs,
) -> String {
    let remote_platform: RemotePlatform = remote_os.into();
    ssh_proxy_lifecycle::service_provider::remote_write_peer_artifact_command(
        artifact,
        remote_platform,
    )
}

pub(crate) fn remote_auto_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_auto_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_systemd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_systemd_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_launchd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_launchd_install_command(&service_spec(
        remote_path,
        args,
    ))
}

pub(crate) fn remote_nohup_start_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
    stop_existing: bool,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_nohup_start_command(
        &service_spec(remote_path, args),
        stop_existing,
    )
}

pub(crate) fn remote_schtasks_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    ssh_proxy_lifecycle::service_provider::remote_schtasks_install_command(&service_spec(
        remote_path,
        args,
    ))
}

fn service_spec(remote_path: &str, args: &cli::InstallRemoteArgs) -> RemotePeerServiceSpec {
    let intent: RemoteInstallIntent = args.into();
    RemotePeerServiceSpec::from_intent(remote_path, &intent)
}
