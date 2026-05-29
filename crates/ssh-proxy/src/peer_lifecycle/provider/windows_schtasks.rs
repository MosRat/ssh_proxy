use crate::cli;

use super::util::{windows_cmd_quote, windows_extra_args};

pub(crate) fn remote_schtasks_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    let token_arg = args
        .remote_token
        .as_deref()
        .map(|token| format!(" --token {}", windows_cmd_quote(token)))
        .unwrap_or_default();
    let command = format!(
        "{} node daemon --transport {} --control tcp://{}{}{}",
        windows_cmd_quote(remote_path),
        args.remote_tcp,
        args.remote_control,
        token_arg,
        windows_extra_args(args)
    );
    format!(
        "schtasks /Create /TN ssh_proxy_helper /SC ONLOGON /RL LIMITED /F /TR {task} && schtasks /Run /TN ssh_proxy_helper && powershell -NoProfile -ExecutionPolicy Bypass -Command \"$dir=Join-Path ([Environment]::GetFolderPath('UserProfile')) '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $now=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds(); Set-Content -LiteralPath (Join-Path $dir 'install_report.json') -Value \\\"{{`\\\\\\\"schema`\\\\\\\":`\\\\\\\"ssh_proxy_remote_install.v1`\\\\\\\",`\\\\\\\"state`\\\\\\\":`\\\\\\\"healthy`\\\\\\\",`\\\\\\\"phase`\\\\\\\":`\\\\\\\"start_service`\\\\\\\",`\\\\\\\"service_manager`\\\\\\\":`\\\\\\\"windows_schtasks_user`\\\\\\\",`\\\\\\\"updated_at_unix`\\\\\\\":$now}}\\\"\"",
        task = windows_cmd_quote(&command),
    )
}
