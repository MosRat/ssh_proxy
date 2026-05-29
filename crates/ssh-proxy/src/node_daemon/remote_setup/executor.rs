use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::Value;
use ssh_proxy_deploy::RemoteSetupExecutionPlan;

use crate::{
    config,
    node_daemon::{proxy_session::ProxySessionSpec, remote_ssh},
    ssh_client,
};

use super::{
    build_cleanup_script, build_git_config_script, cleanup_remote_machine_settings,
    payload::{build_proxy_env, setup_hash, setup_payload},
    write_remote_server_env_setup, write_remote_status_file,
};

#[derive(Debug, Clone, Serialize)]
pub(in crate::node_daemon) struct RemoteSetupOutcome {
    pub(in crate::node_daemon) desired_hash: String,
    pub(in crate::node_daemon) applied_hash: String,
    pub(in crate::node_daemon) remote_url: String,
    pub(in crate::node_daemon) verified: bool,
}

pub(in crate::node_daemon) async fn apply_remote_settings(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
    route: Option<&Value>,
    remote_url: &str,
) -> Result<RemoteSetupOutcome> {
    let install_args = remote_ssh::install_args_from_spec(config, spec)
        .context("failed to build SSH target for remote setup")?;
    let client = ssh_client::Client::connect_install_args(&install_args).await?;
    let execution_plan = RemoteSetupExecutionPlan::new(setup_payload(spec, remote_url, route));
    let payload = &execution_plan.payload;
    let desired_hash = setup_hash(payload);

    if spec.apply_policy.vscode_settings {
        super::apply_remote_machine_settings(&client, payload).await?;
    }

    if spec.apply_policy.server_env {
        write_remote_server_env_setup(
            &client,
            &spec.apply_policy.server_dir,
            &build_proxy_env(remote_url, &spec.apply_policy.no_proxy),
        )
        .await?;
    }

    if spec.apply_policy.git && (spec.apply_policy.git_global || spec.apply_policy.git_workspace) {
        run_script(
            &client,
            &build_git_config_script(
                remote_url,
                &spec.workspace_paths,
                spec.apply_policy.git_global,
                spec.apply_policy.git_workspace,
                spec.apply_policy.git_force_override,
            ),
            "apply remote Git proxy config",
        )
        .await?;
    }

    if spec.apply_policy.remote_status_file {
        write_remote_status_file(&client, &spec.apply_policy.server_dir, payload).await?;
    }

    Ok(RemoteSetupOutcome {
        desired_hash: desired_hash.clone(),
        applied_hash: desired_hash,
        remote_url: remote_url.to_string(),
        verified: false,
    })
}

pub(in crate::node_daemon) async fn cleanup_remote_settings(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
    remote_url: &str,
) -> Result<()> {
    let install_args = remote_ssh::install_args_from_spec(config, spec)
        .context("failed to build SSH target for remote cleanup")?;
    let client = ssh_client::Client::connect_install_args(&install_args).await?;
    cleanup_remote_machine_settings(
        &client,
        &spec.apply_policy.server_dir,
        &[
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ],
    )
    .await?;
    run_script(
        &client,
        &build_cleanup_script(&spec.apply_policy.server_dir, &spec.workspace_paths),
        "cleanup remote proxy settings",
    )
    .await?;
    let _ = remote_url;
    Ok(())
}

async fn run_script(client: &ssh_client::Client, script: &str, label: &str) -> Result<()> {
    let output = client
        .exec_capture("sh -s".to_string(), Some(script.as_bytes().to_vec()))
        .await
        .with_context(|| format!("{label} failed to start"))?;
    if output.exit_status != 0 {
        let stderr = output.stderr.trim();
        let stdout = output.stdout.trim();
        let detail = match (stderr.is_empty(), stdout.is_empty()) {
            (false, false) => format!("stderr: {stderr}; stdout: {stdout}"),
            (false, true) => stderr.to_string(),
            (true, false) => stdout.to_string(),
            (true, true) => "no output".to_string(),
        };
        return Err(anyhow!(
            "{label} failed with status {}: {}",
            output.exit_status,
            detail
        ));
    }
    Ok(())
}
