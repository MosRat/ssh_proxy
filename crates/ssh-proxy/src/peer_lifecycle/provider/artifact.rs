use crate::cli;

use super::super::artifacts::PeerArtifact;

pub(crate) fn remote_write_peer_artifact_command(
    artifact: PeerArtifact,
    remote_os: cli::RemoteOs,
) -> String {
    let name = artifact.file_name();
    match remote_os {
        cli::RemoteOs::Windows => {
            let preserve_existing = if artifact.preserve_existing() {
                "if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $tmp -Force; exit 0 }; "
            } else {
                ""
            };
            format!(
                "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; $home=[Environment]::GetFolderPath('UserProfile'); $dir=Join-Path $home '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $p=Join-Path $dir '{name}'; $tmp=Join-Path $dir ('{name}.tmp.'+[Guid]::NewGuid().ToString('N')); $fs=[IO.File]::Open($tmp,'CreateNew','Write','None'); [Console]::OpenStandardInput().CopyTo($fs); $fs.Close(); {preserve_existing}Move-Item -LiteralPath $tmp -Destination $p -Force\""
            )
        }
        cli::RemoteOs::Unix | cli::RemoteOs::Auto => {
            let preserve_existing = if artifact.preserve_existing() {
                "[ -f \"$p\" ] && { rm -f \"$tmp\"; exit 0; }; "
            } else {
                ""
            };
            let backup_existing = if artifact.backup_existing() {
                "if [ -f \"$p\" ]; then cp \"$p\" \"$HOME/.ssh_proxy/config.toml.bak\" 2>/dev/null || true; fi; "
            } else {
                ""
            };
            format!(
                "set -eu; mkdir -p \"$HOME/.ssh_proxy\"; p=\"$HOME/.ssh_proxy/{name}\"; tmp=\"$p.tmp.$$\"; umask 077; cat > \"$tmp\"; {preserve_existing}{backup_existing}mv \"$tmp\" \"$p\"; chmod 600 \"$p\" 2>/dev/null || true"
            )
        }
    }
}
