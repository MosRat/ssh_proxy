use anyhow::Result;
use serde_json::Value;
pub use ssh_proxy_deploy::RemoteInstallResult;

use crate::{cli, peer_lifecycle, ssh_client};

use super::{
    defaults::apply_remote_auto_defaults, descriptor::wait_remote_peer_descriptor,
    helper::upload_helper, remote_commands::default_persistent_remote_path,
};
use peer_lifecycle::{
    service_provider::PeerServiceProvider,
    workflow::{LifecycleOperation, LifecyclePlan},
};

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

    let (service_manager, install_report) =
        install_remote_service(&client, &remote_path, &args).await?;
    let descriptor = if args.persist == cli::PersistMode::None {
        None
    } else {
        Some(wait_remote_peer_descriptor(&client, &remote_path, &mut args).await?)
    };
    Ok(RemoteInstallResult {
        target: args.target,
        remote_node_id: args.remote_node_id,
        remote_node_name: args.remote_node_name,
        remote_path,
        service_manager,
        remote_tcp: args.remote_tcp,
        remote_control: args.remote_control,
        remote_tls_transport: args.remote_tls_transport,
        remote_quic_transport: args.remote_quic_transport,
        remote_token: args.remote_token,
        descriptor,
        install_report,
    })
}

async fn install_remote_service(
    client: &ssh_client::Client,
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> Result<(String, Option<Value>)> {
    let plan = peer_lifecycle::service_provider::remote_service_install_plan(remote_path, args);
    let spec = peer_lifecycle::spec::PeerLifecycleSpec::remote_peer(
        args.target.clone(),
        remote_path,
        args,
        plan.provider.kind,
    );
    let executor = peer_lifecycle::executor::SshExecutor::new(client);
    let provider = plan.provider.clone();
    match args.persist {
        cli::PersistMode::None => {
            println!("installed helper at {remote_path}");
            println!(
                "use: ssh_proxy proxy {} --remote-path {}",
                args.target, remote_path
            );
            Ok((plan.reported_service_manager, None))
        }
        cli::PersistMode::Auto => {
            let install_report = run_remote_install_plan(
                &executor,
                &spec,
                provider.lifecycle_plan(&spec, LifecycleOperation::Install, Some(plan.command)),
            )
            .await?;
            println!("installed persistent helper on {}", args.target);
            Ok((plan.reported_service_manager, Some(install_report)))
        }
        cli::PersistMode::Systemd => {
            let install_report = run_remote_install_plan(
                &executor,
                &spec,
                provider.lifecycle_plan(&spec, LifecycleOperation::Install, Some(plan.command)),
            )
            .await?;
            println!("installed user systemd service on {}", args.target);
            Ok((plan.reported_service_manager, Some(install_report)))
        }
        cli::PersistMode::Nohup => {
            let install_report = run_remote_install_plan(
                &executor,
                &spec,
                provider.lifecycle_plan(&spec, LifecycleOperation::Install, Some(plan.command)),
            )
            .await?;
            println!("started nohup helper on {}", args.target);
            Ok((plan.reported_service_manager, Some(install_report)))
        }
        cli::PersistMode::Launchd => {
            let install_report = run_remote_install_plan(
                &executor,
                &spec,
                provider.lifecycle_plan(&spec, LifecycleOperation::Install, Some(plan.command)),
            )
            .await?;
            println!("installed user launchd service on {}", args.target);
            Ok((plan.reported_service_manager, Some(install_report)))
        }
        cli::PersistMode::Schtasks => {
            let install_report = run_remote_install_plan(
                &executor,
                &spec,
                provider.lifecycle_plan(&spec, LifecycleOperation::Install, Some(plan.command)),
            )
            .await?;
            println!("installed user scheduled task on {}", args.target);
            Ok((plan.reported_service_manager, Some(install_report)))
        }
    }
}

pub(super) async fn run_remote_install_plan<E: peer_lifecycle::executor::PeerExecutor>(
    executor: &E,
    spec: &peer_lifecycle::spec::PeerLifecycleSpec,
    plan: LifecyclePlan,
) -> Result<Value> {
    let mut sink = peer_lifecycle::workflow::VecLifecycleEventSink::default();
    let result =
        peer_lifecycle::workflow::run_lifecycle_plan(executor, spec, plan, &mut sink).await?;
    Ok(result.report.to_redacted_value())
}
