use std::net::SocketAddr;

use serde_json::json;
use ssh_proxy_core::{
    intent::RemoteInstallIntent,
    model::{PersistenceMode, RemotePlatform},
};

use crate::{
    artifacts::PeerArtifact,
    service_provider::{
        ServiceProviderKind,
        contract::{PeerServiceProvider, ServiceProviderPlan},
        selection::provider_for_platform,
    },
    workflow::{
        LifecycleAction, LifecycleOperation, LifecyclePlan, LifecycleStep, PeerLifecyclePhase,
    },
};

#[derive(Debug, Clone)]
pub struct RemotePeerServiceSpec {
    pub remote_path: String,
    pub remote_platform: RemotePlatform,
    pub persistence: PersistenceMode,
    pub remote_token: Option<String>,
    pub remote_tcp: SocketAddr,
    pub remote_control: SocketAddr,
    pub remote_tls_transport: Option<SocketAddr>,
    pub remote_quic_transport: Option<SocketAddr>,
    pub remote_tls_cert: Option<String>,
    pub remote_tls_key: Option<String>,
    pub remote_tls_client_ca: Option<String>,
}

impl RemotePeerServiceSpec {
    pub fn from_intent(remote_path: impl Into<String>, intent: &RemoteInstallIntent) -> Self {
        Self {
            remote_path: remote_path.into(),
            remote_platform: intent.remote_platform,
            persistence: intent.persistence,
            remote_token: intent.remote_token.clone(),
            remote_tcp: intent.remote_tcp,
            remote_control: intent.remote_control,
            remote_tls_transport: intent.remote_tls_transport,
            remote_quic_transport: intent.remote_quic_transport,
            remote_tls_cert: intent.remote_tls_cert.clone(),
            remote_tls_key: intent.remote_tls_key.clone(),
            remote_tls_client_ca: intent.remote_tls_client_ca.clone(),
        }
    }

    pub fn provider_kind(&self) -> ServiceProviderKind {
        provider_for_platform(self.remote_platform, self.persistence)
    }
}

#[derive(Debug, Clone)]
pub struct ProviderActionPlan {
    pub provider: ServiceProviderPlan,
    pub reported_service_manager: String,
    pub command: String,
    pub lifecycle_plan: LifecyclePlan,
}

pub fn remote_service_action_plan(
    spec: &RemotePeerServiceSpec,
    operation: LifecycleOperation,
) -> ProviderActionPlan {
    let kind = spec.provider_kind();
    let provider = ServiceProviderPlan::new(kind, "ssh-proxy-helper");
    let command = remote_install_command(spec);
    let reported_service_manager =
        reported_service_manager(spec.persistence, spec.remote_platform, kind);
    let mut lifecycle_plan = provider.lifecycle_plan(
        &crate::spec::PeerLifecycleSpec {
            role: crate::spec::PeerLifecycleRole::RemotePeer,
            target: "remote-peer".to_string(),
            platform: remote_platform(spec.remote_platform, spec.persistence),
            scope: remote_scope(spec.persistence),
            provider: kind,
            service_name: "ssh-proxy-helper".to_string(),
            binary_path: spec.remote_path.clone(),
            transport: Some(spec.remote_tcp),
            control_endpoint: Some(format!("tcp://{}", spec.remote_control)),
            token: spec.remote_token.clone(),
            state_dir: remote_state_dir(spec.remote_platform),
            rollback_policy: crate::spec::RollbackPolicy::PreserveExisting,
        },
        operation,
        Some(command.clone()),
    );
    if operation == LifecycleOperation::Install && !command.trim().is_empty() {
        lifecycle_plan = lifecycle_plan
            .push(LifecycleStep::new(
                PeerLifecyclePhase::Record,
                LifecycleAction::WriteArtifact {
                    target: remote_write_peer_artifact_command(
                        PeerArtifact::InstallReport,
                        spec.remote_platform,
                    ),
                    artifact: PeerArtifact::InstallReport,
                    bytes: install_report_bytes(&reported_service_manager),
                },
            ))
            .push(LifecycleStep::new(
                PeerLifecyclePhase::Record,
                LifecycleAction::WriteArtifact {
                    target: remote_write_peer_artifact_command(
                        PeerArtifact::Health,
                        spec.remote_platform,
                    ),
                    artifact: PeerArtifact::Health,
                    bytes: health_report_bytes(&reported_service_manager),
                },
            ));
    }
    ProviderActionPlan {
        provider,
        reported_service_manager,
        command,
        lifecycle_plan,
    }
}

pub fn remote_install_command(spec: &RemotePeerServiceSpec) -> String {
    match spec.persistence {
        PersistenceMode::None => String::new(),
        PersistenceMode::Auto if spec.remote_platform == RemotePlatform::Windows => {
            remote_schtasks_install_command(spec)
        }
        PersistenceMode::Auto => remote_auto_install_command(spec),
        PersistenceMode::Systemd => remote_systemd_install_command(spec),
        PersistenceMode::Nohup => remote_nohup_start_command(spec, true),
        PersistenceMode::Launchd => remote_launchd_install_command(spec),
        PersistenceMode::Schtasks => remote_schtasks_install_command(spec),
    }
}

pub fn remote_auto_install_command(spec: &RemotePeerServiceSpec) -> String {
    format!(
        "set -eu; if [ \"$(uname -s 2>/dev/null || true)\" = Darwin ] && command -v launchctl >/dev/null 2>&1; then {launchd}; elif command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then {systemd}; else {nohup}; fi",
        launchd = remote_launchd_install_command(spec),
        systemd = remote_systemd_install_command(spec),
        nohup = remote_nohup_start_command(spec, true)
    )
}

pub fn remote_systemd_install_command(spec: &RemotePeerServiceSpec) -> String {
    let escaped = sh_quote(&spec.remote_path);
    let token_arg = token_arg(spec.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(spec);
    format!(
        "set -eu; systemctl --user show-environment >/dev/null; if command -v loginctl >/dev/null 2>&1; then loginctl enable-linger \"$(id -un)\" >/dev/null 2>&1 || true; fi; mkdir -p ~/.config/systemd/user; cat > ~/.config/systemd/user/ssh-proxy-helper.service <<'EOF'\n[Unit]\nDescription=ssh_proxy node daemon\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=0\n[Service]\nExecStart={} node daemon --transport {} --control tcp://{}{}{}\nRestart=always\nRestartSec=3\nKillSignal=SIGINT\n[Install]\nWantedBy=default.target\nEOF\nsystemctl --user daemon-reload && systemctl --user enable ssh-proxy-helper.service && systemctl --user restart ssh-proxy-helper.service",
        escaped, spec.remote_tcp, spec.remote_control, token_arg, extra_args
    )
}

pub fn remote_launchd_install_command(spec: &RemotePeerServiceSpec) -> String {
    let token_arg = token_arg(spec.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(spec);
    let plist = "$HOME/Library/LaunchAgents/com.ssh-proxy.helper.plist";
    format!(
        "set -eu; mkdir -p \"$HOME/Library/LaunchAgents\"; cat > {plist} <<'EOF'\n<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>com.ssh-proxy.helper</string>\n<key>ProgramArguments</key><array><string>{remote_path}</string><string>node</string><string>daemon</string><string>--transport</string><string>{remote_tcp}</string><string>--control</string><string>tcp://{remote_control}</string>{token_plist}{extra_plist}</array>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>StandardOutPath</key><string>{home}/.ssh_proxy/log/launchd.log</string>\n<key>StandardErrorPath</key><string>{home}/.ssh_proxy/log/launchd.log</string>\n</dict></plist>\nEOF\nmkdir -p \"$HOME/.ssh_proxy/log\"; launchctl bootout gui/$(id -u) {plist} >/dev/null 2>&1 || true; launchctl bootstrap gui/$(id -u) {plist}; launchctl kickstart -k gui/$(id -u)/com.ssh-proxy.helper",
        remote_path = xml_escape(&spec.remote_path),
        remote_tcp = spec.remote_tcp,
        remote_control = spec.remote_control,
        token_plist = token_arg_to_plist(&token_arg),
        extra_plist = extra_args_to_plist(&extra_args),
        home = "$HOME",
    )
}

pub fn remote_nohup_start_command(spec: &RemotePeerServiceSpec, stop_existing: bool) -> String {
    let stop = if stop_existing {
        format!("{}; ", remote_nohup_stop_snippet(spec.remote_tcp))
    } else {
        String::new()
    };
    let token_arg = token_arg(spec.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(spec);
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(spec.remote_tcp);
    format!(
        "set -eu; mkdir -p \"$HOME/.ssh_proxy/run\" \"$HOME/.ssh_proxy/log\"; {stop} cat > {scriptfile} <<'EOF'\n#!/bin/sh\nset -u\ntrap 'if [ -f {childfile} ]; then child=$(cat {childfile} 2>/dev/null || true); [ -n \"$child\" ] && kill \"$child\" 2>/dev/null || true; fi; exit 0' INT TERM\nbackoff=1\nwhile :; do\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) starting ssh_proxy node daemon\" >>{logfile}\n  {remote_path} node daemon --transport {remote_tcp} --control tcp://{remote_control}{token_arg}{extra_args} >>{logfile} 2>&1 &\n  child=$!\n  echo \"$child\" >{childfile}\n  wait \"$child\"\n  code=$?\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) ssh_proxy node daemon exited status=$code; restarting in ${{backoff}}s\" >>{logfile}\n  sleep \"$backoff\"\n  if [ \"$backoff\" -lt 30 ]; then backoff=$((backoff * 2)); fi\ndone\nEOF\nchmod 700 {scriptfile}; nohup /bin/sh {scriptfile} >/dev/null 2>&1 < /dev/null & echo $! > {pidfile}; sleep 1; pid=$(cat {pidfile}); kill -0 \"$pid\"",
        remote_path = sh_quote(&spec.remote_path),
        remote_tcp = spec.remote_tcp,
        remote_control = spec.remote_control,
    )
}

pub fn remote_schtasks_install_command(spec: &RemotePeerServiceSpec) -> String {
    let token_arg = spec
        .remote_token
        .as_deref()
        .map(|token| format!(" --token {}", windows_cmd_quote(token)))
        .unwrap_or_default();
    let command = format!(
        "{} node daemon --transport {} --control tcp://{}{}{}",
        windows_cmd_quote(&spec.remote_path),
        spec.remote_tcp,
        spec.remote_control,
        token_arg,
        windows_extra_args(spec)
    );
    format!(
        "schtasks /Create /TN ssh_proxy_helper /SC ONLOGON /RL LIMITED /F /TR {task} && schtasks /Run /TN ssh_proxy_helper",
        task = windows_cmd_quote(&command),
    )
}

pub fn remote_write_peer_artifact_command(
    artifact: PeerArtifact,
    remote_platform: RemotePlatform,
) -> String {
    let name = artifact.file_name();
    match remote_platform {
        RemotePlatform::Windows => {
            let preserve_existing = if artifact.preserve_existing() {
                "if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $tmp -Force; exit 0 }; "
            } else {
                ""
            };
            format!(
                "powershell -NoProfile -ExecutionPolicy Bypass -Command \"$ErrorActionPreference='Stop'; $home=[Environment]::GetFolderPath('UserProfile'); $dir=Join-Path $home '.ssh_proxy'; New-Item -ItemType Directory -Force -Path $dir | Out-Null; $p=Join-Path $dir '{name}'; $tmp=Join-Path $dir ('{name}.tmp.'+[Guid]::NewGuid().ToString('N')); $fs=[IO.File]::Open($tmp,'CreateNew','Write','None'); [Console]::OpenStandardInput().CopyTo($fs); $fs.Close(); {preserve_existing}Move-Item -LiteralPath $tmp -Destination $p -Force\""
            )
        }
        RemotePlatform::Unix | RemotePlatform::Auto => {
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

pub fn remote_nohup_status_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; logfile={}; scriptfile={}; if [ -f \"$pidfile\" ]; then pid=$(cat \"$pidfile\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then echo \"supervisor running pid=$pid\"; else echo \"stale supervisor pidfile pid=$pid\"; fi; else echo \"not installed\"; fi; if [ -f \"$childfile\" ]; then child=$(cat \"$childfile\" 2>/dev/null || true); if [ -n \"$child\" ] && kill -0 \"$child\" 2>/dev/null; then echo \"helper running pid=$child\"; else echo \"stale helper child pid=$child\"; fi; fi; if [ -f \"$logfile\" ]; then echo \"log=$logfile\"; tail -n 20 \"$logfile\" 2>/dev/null || true; fi",
        pidfile, childfile, logfile, scriptfile
    )
}

pub fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, _, _) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; for f in \"$childfile\" \"$pidfile\"; do if [ -f \"$f\" ]; then pid=$(cat \"$f\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\" 2>/dev/null || true; for i in 1 2 3 4 5; do kill -0 \"$pid\" 2>/dev/null || break; sleep 1; done; kill -9 \"$pid\" 2>/dev/null || true; fi; rm -f \"$f\"; fi; done",
        pidfile, childfile
    )
}

pub fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    let port = remote_tcp.port();
    (
        format!("$HOME/.ssh_proxy/run/helper-{port}.pid"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.child.pid"),
        format!("$HOME/.ssh_proxy/log/helper-{port}.log"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.supervisor.sh"),
    )
}

fn reported_service_manager(
    persistence: PersistenceMode,
    platform: RemotePlatform,
    kind: ServiceProviderKind,
) -> String {
    match persistence {
        PersistenceMode::None => "none",
        PersistenceMode::Auto if platform == RemotePlatform::Windows => {
            ServiceProviderKind::WindowsScheduledTaskUser.manager_name()
        }
        PersistenceMode::Auto => "auto",
        _ => kind.manager_name(),
    }
    .to_string()
}

fn install_report_bytes(service_manager: &str) -> Vec<u8> {
    json!({
        "schema": "ssh_proxy_remote_install.v1",
        "state": "healthy",
        "phase": "start_service",
        "service_manager": service_manager,
        "updated_at_unix": now_unix(),
    })
    .to_string()
    .into_bytes()
}

fn health_report_bytes(service_manager: &str) -> Vec<u8> {
    json!({
        "schema": "ssh_proxy_peer_health.v1",
        "state": "healthy",
        "service_manager": service_manager,
        "updated_at_unix": now_unix(),
    })
    .to_string()
    .into_bytes()
}

fn node_daemon_extra_args(spec: &RemotePeerServiceSpec) -> String {
    let mut out = String::new();
    if let Some(addr) = spec.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = spec.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    if let Some(path) = &spec.remote_tls_cert {
        out.push_str(&format!(" --tls-cert {}", sh_quote(path)));
    }
    if let Some(path) = &spec.remote_tls_key {
        out.push_str(&format!(" --tls-key {}", sh_quote(path)));
    }
    if let Some(path) = &spec.remote_tls_client_ca {
        out.push_str(&format!(" --tls-client-ca {}", sh_quote(path)));
    }
    out
}

fn windows_extra_args(spec: &RemotePeerServiceSpec) -> String {
    let mut out = String::new();
    if let Some(addr) = spec.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = spec.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    out
}

fn token_arg(token: Option<&str>) -> String {
    token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default()
}

fn token_arg_to_plist(token_arg: &str) -> String {
    if token_arg.is_empty() {
        return String::new();
    }
    token_arg
        .split_whitespace()
        .map(|part| format!("<string>{}</string>", xml_escape(part.trim_matches('\''))))
        .collect::<Vec<_>>()
        .join("")
}

fn extra_args_to_plist(extra_args: &str) -> String {
    extra_args
        .split_whitespace()
        .map(|part| format!("<string>{}</string>", xml_escape(part.trim_matches('\''))))
        .collect::<Vec<_>>()
        .join("")
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn windows_cmd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn remote_platform(
    platform: RemotePlatform,
    persistence: PersistenceMode,
) -> crate::spec::PeerLifecyclePlatform {
    match persistence {
        PersistenceMode::Launchd => crate::spec::PeerLifecyclePlatform::Macos,
        PersistenceMode::Schtasks => crate::spec::PeerLifecyclePlatform::Windows,
        _ => match platform {
            RemotePlatform::Windows => crate::spec::PeerLifecyclePlatform::Windows,
            RemotePlatform::Unix => crate::spec::PeerLifecyclePlatform::Linux,
            RemotePlatform::Auto => crate::spec::PeerLifecyclePlatform::Unknown,
        },
    }
}

fn remote_scope(persistence: PersistenceMode) -> crate::spec::PeerLifecycleScope {
    match persistence {
        PersistenceMode::None => crate::spec::PeerLifecycleScope::None,
        PersistenceMode::Nohup => crate::spec::PeerLifecycleScope::Managed,
        _ => crate::spec::PeerLifecycleScope::User,
    }
}

fn remote_state_dir(platform: RemotePlatform) -> String {
    match platform {
        RemotePlatform::Windows => "%USERPROFILE%\\.ssh_proxy".to_string(),
        RemotePlatform::Unix | RemotePlatform::Auto => "$HOME/.ssh_proxy".to_string(),
    }
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use ssh_proxy_core::{intent::SshTargetIntent, model::PersistenceMode};

    use super::*;

    fn install_spec(persistence: PersistenceMode) -> RemotePeerServiceSpec {
        let mut intent = RemoteInstallIntent::new(
            SshTargetIntent::new("edge"),
            "127.0.0.1:19080".parse().unwrap(),
            "127.0.0.1:19081".parse().unwrap(),
            persistence,
        );
        intent.remote_token = Some("secret".to_string());
        intent.remote_platform = RemotePlatform::Unix;
        RemotePeerServiceSpec::from_intent("/home/me/bin/ssh_proxy", &intent)
    }

    #[test]
    fn provider_commands_do_not_embed_lifecycle_json() {
        let spec = install_spec(PersistenceMode::Systemd);
        let command = remote_systemd_install_command(&spec);

        assert!(command.contains("systemctl --user daemon-reload"));
        assert!(command.contains("ExecStart='/home/me/bin/ssh_proxy' node daemon"));
        assert!(!command.contains("install_report.json"));
        assert!(!command.contains("\"service_manager\""));
    }

    #[test]
    fn provider_action_plan_records_artifacts_as_actions() {
        let spec = install_spec(PersistenceMode::Systemd);
        let plan = remote_service_action_plan(&spec, LifecycleOperation::Install);

        assert_eq!(plan.reported_service_manager, "systemd_user");
        assert_eq!(plan.provider.kind.manager_name(), "systemd_user");
        assert!(plan.lifecycle_plan.steps.iter().any(|step| matches!(
            step.action,
            LifecycleAction::WriteArtifact {
                artifact: PeerArtifact::InstallReport,
                ..
            }
        )));
        assert!(plan.lifecycle_plan.steps.iter().any(|step| matches!(
            step.action,
            LifecycleAction::WriteArtifact {
                artifact: PeerArtifact::Health,
                ..
            }
        )));
    }

    #[test]
    fn remote_artifact_write_uses_stdin_and_no_heredoc_payload() {
        let command =
            remote_write_peer_artifact_command(PeerArtifact::Config, RemotePlatform::Unix);

        assert!(command.contains("cat > \"$tmp\""));
        assert!(command.contains("config.toml.bak"));
        assert!(!command.contains("<<"));
        assert!(!command.contains("node_id ="));
    }
}
