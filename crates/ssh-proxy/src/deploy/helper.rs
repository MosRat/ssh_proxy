use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use ssh_proxy_core::model::RemotePlatform;
use ssh_proxy_deploy::{RemoteAdminChecksumReport, RemoteAdminIntent, remote_admin_stdin_command};
use tracing::{info, warn};

use crate::{cli, sidecar, ssh_client};

use super::remote_commands::sh_quote;

#[derive(Debug, Clone, Copy)]
pub enum HelperCapability {
    Stdio,
    ReverseSocks { listen: SocketAddr },
}

pub async fn ensure_helper(
    args: &cli::ProxyArgs,
    client: &ssh_client::Client,
    capability: HelperCapability,
) -> Result<String> {
    if args.deploy == cli::DeployMode::Never {
        return Ok(args
            .remote_path
            .clone()
            .unwrap_or_else(|| "ssh_proxy".to_string()));
    }
    if let Some(path) = &args.remote_path {
        if args.deploy == cli::DeployMode::Auto {
            match probe_helper(client, path, args.remote_os, capability).await {
                Ok(()) => return Ok(path.clone()),
                Err(err) => {
                    warn!(
                        remote_path = %path,
                        error = %err,
                        "remote helper probe failed; uploading a fresh helper"
                    );
                }
            }
        }
    }
    upload_helper(
        client,
        args.remote_bin.as_ref(),
        args.remote_path.as_deref(),
        args.remote_os,
    )
    .await
}

async fn probe_helper(
    client: &ssh_client::Client,
    remote_path: &str,
    remote_os: cli::RemoteOs,
    capability: HelperCapability,
) -> Result<()> {
    let remote_os = match remote_os {
        cli::RemoteOs::Auto => cli::RemoteOs::Unix,
        other => other,
    };
    let command = helper_probe_command(remote_path, remote_os, capability);
    client
        .exec_output(command)
        .await
        .map(|_| ())
        .with_context(|| format!("remote helper {remote_path} is missing or incompatible"))
}

fn helper_probe_command(
    remote_path: &str,
    remote_os: cli::RemoteOs,
    capability: HelperCapability,
) -> String {
    match remote_os {
        cli::RemoteOs::Windows => match capability {
            HelperCapability::Stdio => format!("{remote_path} remote --help >NUL"),
            HelperCapability::ReverseSocks { .. } => {
                format!("{remote_path} remote --help | findstr /C:\"--reverse-socks\" >NUL")
            }
        },
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => {
            let path = sh_quote(remote_path);
            match capability {
                HelperCapability::Stdio => {
                    format!("{path} remote --help >/dev/null")
                }
                HelperCapability::ReverseSocks { listen } => {
                    let port = listen.port();
                    format!(
                        "set -eu; {path} remote --help 2>&1 | grep -q -- '--reverse-socks'; if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | awk '{{print $4}}' | grep -Eq '(:|\\]){port}$'; then echo 'remote reverse listen port {port} is already in use' >&2; exit 75; fi"
                    )
                }
            }
        }
    }
}

pub async fn upload_helper(
    client: &ssh_client::Client,
    local_bin: Option<&PathBuf>,
    remote_path: Option<&str>,
    remote_os: cli::RemoteOs,
) -> Result<String> {
    let local_path = match local_bin {
        Some(path) => path.clone(),
        None => upload_binary_path(remote_os)?,
    };
    let bytes = tokio::fs::read(&local_path)
        .await
        .with_context(|| format!("failed to read local helper {}", local_path.display()))?;
    let remote_os = match remote_os {
        cli::RemoteOs::Auto => cli::RemoteOs::Unix,
        other => other,
    };
    let remote_path = remote_path
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_remote_path(remote_os));

    let local_sha256 = sha256_hex(&bytes);
    if remote_helper_matches(client, &remote_path, remote_os, &local_sha256, bytes.len()).await? {
        info!(
            %remote_path,
            bytes = bytes.len(),
            sha256 = %local_sha256,
            "remote helper already matches local sidecar; skipping upload"
        );
        return Ok(remote_path);
    }

    info!(
        %remote_path,
        bytes = bytes.len(),
        sha256 = %local_sha256,
        "uploading remote helper"
    );
    match remote_os {
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => {
            let command = unix_upload_helper_command(&remote_path);
            client.exec_upload(command, bytes).await?;
        }
        cli::RemoteOs::Windows => {
            let escaped = remote_path.replace('\'', "''");
            let command = format!(
                "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$p=[Environment]::ExpandEnvironmentVariables('{}'); $d=Split-Path -Parent $p; if ($d) {{ New-Item -ItemType Directory -Force -Path $d | Out-Null }}; $fs=[IO.File]::Open($p,'Create','Write','None'); [Console]::OpenStandardInput().CopyTo($fs); $fs.Close()\"",
                escaped
            );
            client.exec_upload(command, bytes).await?;
        }
    }
    Ok(remote_path)
}

fn unix_upload_helper_command(remote_path: &str) -> String {
    format!(
        "set -eu; p={}; mkdir -p \"$(dirname \"$p\")\"; tmp=\"$p.tmp.$$\"; trap 'rm -f \"$tmp\"' EXIT INT TERM; cat > \"$tmp\"; chmod 700 \"$tmp\"; mv -f \"$tmp\" \"$p\"; trap - EXIT",
        sh_quote(remote_path)
    )
}

async fn remote_helper_matches(
    client: &ssh_client::Client,
    remote_path: &str,
    remote_os: cli::RemoteOs,
    local_sha256: &str,
    local_len: usize,
) -> Result<bool> {
    match remote_helper_checksum_via_admin(client, remote_path, remote_os).await {
        Ok(report) => {
            return Ok(
                report.sha256.eq_ignore_ascii_case(local_sha256) && report.len == local_len as u64
            );
        }
        Err(err) => {
            info!(
                %remote_path,
                error = %err,
                "remote helper admin checksum probe failed; falling back to shell checksum probe"
            );
        }
    }

    let command = match remote_os {
        cli::RemoteOs::Windows => {
            let escaped = remote_path.replace('\'', "''");
            format!(
                "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$p=[Environment]::ExpandEnvironmentVariables('{}'); if (!(Test-Path -LiteralPath $p)) {{ exit 2 }}; $h=(Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLowerInvariant(); $l=(Get-Item -LiteralPath $p).Length; Write-Output \\\"$h $l\\\"\"",
                escaped
            )
        }
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => format!(
            "set -eu; p={}; [ -f \"$p\" ] || exit 2; if command -v sha256sum >/dev/null 2>&1; then h=$(sha256sum \"$p\" | awk '{{print $1}}'); elif command -v shasum >/dev/null 2>&1; then h=$(shasum -a 256 \"$p\" | awk '{{print $1}}'); else exit 3; fi; l=$(wc -c <\"$p\" | tr -d ' '); printf '%s %s\\n' \"$h\" \"$l\"",
            sh_quote(remote_path)
        ),
    };

    let output = match client.exec_output(command).await {
        Ok(output) => output,
        Err(err) => {
            info!(%remote_path, error = %err, "remote helper checksum probe failed");
            return Ok(false);
        }
    };
    let mut parts = output.split_whitespace();
    let remote_sha256 = parts.next().unwrap_or_default();
    let remote_len = parts
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    Ok(remote_sha256.eq_ignore_ascii_case(local_sha256) && remote_len == local_len)
}

async fn remote_helper_checksum_via_admin(
    client: &ssh_client::Client,
    remote_path: &str,
    remote_os: cli::RemoteOs,
) -> Result<RemoteAdminChecksumReport> {
    let remote_platform: RemotePlatform = remote_os.into();
    let command = remote_admin_stdin_command(remote_path, remote_platform);
    let intent = RemoteAdminIntent::Checksum {
        path: remote_path.to_string(),
    };
    let stdin = serde_json::to_vec(&intent).context("failed to encode remote admin checksum")?;
    let output = client.exec_capture(command, Some(stdin)).await?;
    if output.exit_status != 0 {
        anyhow::bail!(
            "remote admin checksum exited with status {}: {}",
            output.exit_status,
            output.stderr.trim()
        );
    }
    let response: serde_json::Value = serde_json::from_str(&output.stdout)
        .context("remote admin checksum did not return JSON")?;
    if !response["ok"].as_bool().unwrap_or(false) {
        anyhow::bail!(
            "remote admin checksum failed: {}",
            response["error"].as_str().unwrap_or("unknown error")
        );
    }
    serde_json::from_value(response["data"].clone())
        .context("remote admin checksum JSON has invalid data")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn default_remote_path(remote_os: cli::RemoteOs) -> String {
    let arch = std::env::consts::ARCH;
    match remote_os {
        cli::RemoteOs::Windows => format!(r"%TEMP%\ssh_proxy-{arch}.exe"),
        _ => format!("/tmp/ssh_proxy-{arch}"),
    }
}

fn upload_binary_path(remote_os: cli::RemoteOs) -> Result<PathBuf> {
    let current = std::env::current_exe().context("failed to locate current executable")?;
    let wants_unix = matches!(remote_os, cli::RemoteOs::Unix | cli::RemoteOs::Auto);
    if wants_unix && std::env::consts::OS != "linux" {
        if let Some(path) = sidecar::materialize_linux_musl()? {
            info!(path = %path.display(), "using embedded linux-musl helper sidecar");
            return Ok(path);
        }
        let arch = std::env::consts::ARCH;
        let sidecar_name = format!("ssh_proxy-{arch}-unknown-linux-musl");
        if let Some(parent) = current.parent() {
            let sidecar = parent.join(&sidecar_name);
            if sidecar.exists() {
                info!(path = %sidecar.display(), "using carried linux-musl helper sidecar");
                return Ok(sidecar);
            }
        }
        let asset = PathBuf::from("assets").join(&sidecar_name);
        if asset.exists() {
            info!(path = %asset.display(), "using linux-musl helper asset");
            return Ok(asset);
        }
        warn!(
            sidecar = %sidecar_name,
            "no linux-musl helper sidecar found; uploading the current executable"
        );
    }
    Ok(current)
}

pub fn remote_stdio_command(remote_path: &str, remote_os: cli::RemoteOs) -> String {
    match remote_os {
        cli::RemoteOs::Windows => format!("{} remote --stdio", remote_path),
        _ => format!("{} remote --stdio", sh_quote(remote_path)),
    }
}

pub fn remote_reverse_socks_command(
    remote_path: &str,
    remote_os: cli::RemoteOs,
    remote_listen: SocketAddr,
) -> String {
    match remote_os {
        cli::RemoteOs::Windows => {
            format!("{remote_path} remote --stdio --reverse-socks {remote_listen}")
        }
        _ => format!(
            "{} remote --stdio --reverse-socks {}",
            sh_quote(remote_path),
            remote_listen
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_upload_helper_replaces_running_binary_via_temp_file() {
        let command = unix_upload_helper_command("/home/me/.local/bin/ssh_proxy");

        assert!(command.contains("tmp=\"$p.tmp.$$\""), "{command}");
        assert!(command.contains("cat > \"$tmp\""), "{command}");
        assert!(command.contains("mv -f \"$tmp\" \"$p\""), "{command}");
        assert!(!command.contains("cat > \"$p\""), "{command}");
    }
}
