use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::Value;
use ssh_proxy_deploy::{RemoteAdminIntent, RemoteSetupExecutionPlan};
use tracing::info;

use crate::{
    config,
    node_daemon::{proxy_session::ProxySessionSpec, remote_ssh},
    ssh_client,
};

use super::{
    build_cleanup_script_with_git, build_git_config_script, cleanup_remote_machine_settings,
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

    if spec.apply_policy.git
        && (spec.apply_policy.git_global || spec.apply_policy.git_workspace)
        && !try_apply_git_config_with_helper(
            &client,
            remote_url,
            &spec.workspace_paths,
            spec.apply_policy.git_global,
            spec.apply_policy.git_workspace,
            spec.apply_policy.git_force_override,
        )
        .await?
    {
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
    let git_cleanup_done = if spec.apply_policy.git
        && (spec.apply_policy.git_global || spec.apply_policy.git_workspace)
    {
        try_cleanup_git_config_with_helper(
            &client,
            &spec.workspace_paths,
            spec.apply_policy.git_global,
            spec.apply_policy.git_workspace,
        )
        .await?
    } else {
        false
    };
    run_script(
        &client,
        &build_cleanup_script_with_git(
            &spec.apply_policy.server_dir,
            &spec.workspace_paths,
            !git_cleanup_done,
        ),
        "cleanup remote proxy settings",
    )
    .await?;
    let _ = remote_url;
    Ok(())
}

async fn try_apply_git_config_with_helper(
    client: &ssh_client::Client,
    proxy_url: &str,
    workspace_paths: &[String],
    apply_global: bool,
    apply_workspace: bool,
    force_override: bool,
) -> Result<bool> {
    if !force_override {
        return Ok(false);
    }
    let mut intents = Vec::new();
    if apply_global {
        intents.push(RemoteAdminIntent::GitApply {
            config_path: Some("~/.gitconfig".to_string()),
            workspace_path: None,
            http_proxy: Some(proxy_url.to_string()),
            https_proxy: Some(proxy_url.to_string()),
        });
    }
    if apply_workspace {
        for workspace_path in workspace_paths {
            intents.push(RemoteAdminIntent::GitApply {
                config_path: None,
                workspace_path: Some(workspace_path.clone()),
                http_proxy: Some(proxy_url.to_string()),
                https_proxy: Some(proxy_url.to_string()),
            });
        }
    }
    run_remote_admin_intents(client, intents, "apply remote Git proxy config").await
}

async fn try_cleanup_git_config_with_helper(
    client: &ssh_client::Client,
    workspace_paths: &[String],
    cleanup_global: bool,
    cleanup_workspace: bool,
) -> Result<bool> {
    let mut intents = Vec::new();
    if cleanup_global {
        intents.push(RemoteAdminIntent::GitCleanup {
            config_path: Some("~/.gitconfig".to_string()),
            workspace_path: None,
        });
    }
    if cleanup_workspace {
        for workspace_path in workspace_paths {
            intents.push(RemoteAdminIntent::GitCleanup {
                config_path: None,
                workspace_path: Some(workspace_path.clone()),
            });
        }
    }
    run_remote_admin_intents(client, intents, "cleanup remote Git proxy config").await
}

async fn run_remote_admin_intents(
    client: &ssh_client::Client,
    intents: Vec<RemoteAdminIntent>,
    label: &str,
) -> Result<bool> {
    if intents.is_empty() {
        return Ok(true);
    }
    for intent in intents {
        let stdin = serde_json::to_vec(&intent).context("failed to encode remote admin intent")?;
        let output = client
            .exec_capture("ssh_proxy remote admin".to_string(), Some(stdin))
            .await;
        let output = match output {
            Ok(output) => output,
            Err(err) => {
                info!(error = %err, "{label} helper unavailable; falling back to script");
                return Ok(false);
            }
        };
        if output.exit_status != 0 {
            info!(
                status = output.exit_status,
                stderr = %output.stderr.trim(),
                "{label} helper failed; falling back to script"
            );
            return Ok(false);
        }
        let response: Value = match serde_json::from_str(&output.stdout) {
            Ok(response) => response,
            Err(err) => {
                info!(error = %err, "{label} helper returned invalid JSON; falling back to script");
                return Ok(false);
            }
        };
        if !response["ok"].as_bool().unwrap_or(false) {
            info!(
                error = response["error"].as_str().unwrap_or("unknown error"),
                "{label} helper reported failure; falling back to script"
            );
            return Ok(false);
        }
    }
    Ok(true)
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
