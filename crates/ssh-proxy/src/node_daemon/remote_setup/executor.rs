use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::Value;
use ssh_proxy_deploy::{
    RemoteAdminIntent, RemoteSetupExecutionPlan, RemoteSetupScriptIntent,
    build_cleanup_script_with_git, build_git_config_script,
};
use tracing::info;

use crate::{
    config,
    node_daemon::{proxy_session::ProxySessionSpec, remote_ssh},
    ssh_client,
};

use super::{
    cleanup_remote_machine_settings,
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
        && !try_apply_git_config_with_helper(&client, spec, remote_url).await?
    {
        run_script(
            &client,
            spec,
            &RemoteSetupScriptIntent::fallback_shell("apply remote Git proxy config"),
            &build_git_config_script(
                remote_url,
                &spec.workspace_paths,
                spec.apply_policy.git_global,
                spec.apply_policy.git_workspace,
                spec.apply_policy.git_force_override,
            ),
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
        try_cleanup_git_config_with_helper(&client, spec).await?
    } else {
        false
    };
    run_script(
        &client,
        spec,
        &RemoteSetupScriptIntent::fallback_shell("cleanup remote proxy settings"),
        &build_cleanup_script_with_git(
            &spec.apply_policy.server_dir,
            &spec.workspace_paths,
            !git_cleanup_done,
        ),
    )
    .await?;
    let _ = remote_url;
    Ok(())
}

async fn try_apply_git_config_with_helper(
    client: &ssh_client::Client,
    spec: &ProxySessionSpec,
    proxy_url: &str,
) -> Result<bool> {
    if !spec.apply_policy.git_force_override {
        return Ok(false);
    }
    let mut intents = Vec::new();
    if spec.apply_policy.git_global {
        intents.push(RemoteAdminIntent::GitApply {
            config_path: Some("~/.gitconfig".to_string()),
            workspace_path: None,
            http_proxy: Some(proxy_url.to_string()),
            https_proxy: Some(proxy_url.to_string()),
        });
    }
    if spec.apply_policy.git_workspace {
        for workspace_path in &spec.workspace_paths {
            intents.push(RemoteAdminIntent::GitApply {
                config_path: None,
                workspace_path: Some(workspace_path.clone()),
                http_proxy: Some(proxy_url.to_string()),
                https_proxy: Some(proxy_url.to_string()),
            });
        }
    }
    run_remote_admin_intents(client, spec, intents, "apply remote Git proxy config").await
}

async fn try_cleanup_git_config_with_helper(
    client: &ssh_client::Client,
    spec: &ProxySessionSpec,
) -> Result<bool> {
    let mut intents = Vec::new();
    if spec.apply_policy.git_global {
        intents.push(RemoteAdminIntent::GitCleanup {
            config_path: Some("~/.gitconfig".to_string()),
            workspace_path: None,
        });
    }
    if spec.apply_policy.git_workspace {
        for workspace_path in &spec.workspace_paths {
            intents.push(RemoteAdminIntent::GitCleanup {
                config_path: None,
                workspace_path: Some(workspace_path.clone()),
            });
        }
    }
    run_remote_admin_intents(client, spec, intents, "cleanup remote Git proxy config").await
}

async fn run_remote_admin_intents(
    client: &ssh_client::Client,
    spec: &ProxySessionSpec,
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
                info!(
                    job_id = %spec.job_id(),
                    session_id = %spec.session_id(),
                    route_id = %spec.route_id(),
                    peer = %spec.target,
                    execution_backend = "remote_shell_bootstrap",
                    fallback_used = true,
                    error = %err,
                    "{label} helper unavailable; falling back to script"
                );
                return Ok(false);
            }
        };
        if output.exit_status != 0 {
            info!(
                job_id = %spec.job_id(),
                session_id = %spec.session_id(),
                route_id = %spec.route_id(),
                peer = %spec.target,
                execution_backend = "remote_shell_bootstrap",
                fallback_used = true,
                status = output.exit_status,
                stderr = %output.stderr.trim(),
                "{label} helper failed; falling back to script"
            );
            return Ok(false);
        }
        let response: Value = match serde_json::from_str(&output.stdout) {
            Ok(response) => response,
            Err(err) => {
                info!(
                    job_id = %spec.job_id(),
                    session_id = %spec.session_id(),
                    route_id = %spec.route_id(),
                    peer = %spec.target,
                    execution_backend = "remote_shell_bootstrap",
                    fallback_used = true,
                    error = %err,
                    "{label} helper returned invalid JSON; falling back to script"
                );
                return Ok(false);
            }
        };
        if !response["ok"].as_bool().unwrap_or(false) {
            info!(
                job_id = %spec.job_id(),
                session_id = %spec.session_id(),
                route_id = %spec.route_id(),
                peer = %spec.target,
                execution_backend = "remote_shell_bootstrap",
                fallback_used = true,
                error = response["error"].as_str().unwrap_or("unknown error"),
                "{label} helper reported failure; falling back to script"
            );
            return Ok(false);
        }
    }
    Ok(true)
}

async fn run_script(
    client: &ssh_client::Client,
    spec: &ProxySessionSpec,
    intent: &RemoteSetupScriptIntent,
    script: &str,
) -> Result<()> {
    let external_action = intent.external_action_report();
    let external_action_json = external_action.to_json();
    let output = client
        .exec_capture(intent.command.clone(), Some(script.as_bytes().to_vec()))
        .await
        .with_context(|| format!("{} failed to start", intent.label))?;
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
            "{} failed with status {} as {}: {}",
            intent.label,
            output.exit_status,
            intent.class.as_str(),
            detail
        ));
    }
    info!(
        job_id = %spec.job_id(),
        session_id = %spec.session_id(),
        route_id = %spec.route_id(),
        peer = %spec.target,
        label = %intent.label,
        class = %intent.class.as_str(),
        execution_backend = %external_action.execution_backend,
        fallback_used = external_action.fallback_used,
        external_action = %external_action_json,
        "remote setup fallback script completed"
    );
    Ok(())
}
