use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{cli, deploy, peer_lifecycle};

use super::{PeerStatusRecord, phase_mapping::lifecycle_phase_from_install_state};

pub(super) fn peer_status_from_descriptor(
    alias: &str,
    descriptor: &Value,
    remote_control: std::net::SocketAddr,
    remote_tcp: std::net::SocketAddr,
    install_state: &str,
    install_phase: &str,
    service_manager: &str,
    recovery_attempts: u32,
    install_args: Option<&cli::InstallRemoteArgs>,
    remote_path: Option<&str>,
    operation: peer_lifecycle::workflow::LifecycleOperation,
) -> PeerStatusRecord {
    let protocols = descriptor
        .get("transport_protocols")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec!["plain-tcp".to_string()]);
    PeerStatusRecord {
        target: alias.to_string(),
        state: "healthy".to_string(),
        health: "healthy".to_string(),
        version: descriptor
            .get("version")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        control_endpoint: descriptor
            .pointer("/endpoints/control")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| Some(format!("tcp://{remote_control}"))),
        transport: descriptor
            .pointer("/endpoints/transport")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| Some(remote_tcp.to_string())),
        transport_protocols: protocols,
        service_manager: Some(service_manager.to_string()),
        descriptor_hash: Some(value_hash(descriptor)),
        install: Some(remote_peer_lifecycle_report(
            alias,
            lifecycle_phase_from_install_state(install_state, install_phase),
            operation,
            install_args,
            remote_path,
            service_manager,
            None,
            None,
            recovery_attempts,
        )),
        dependency_report: Some(remote_dependency_report()),
        update_required: descriptor
            .get("version")
            .and_then(Value::as_str)
            .is_some_and(|version| version != env!("CARGO_PKG_VERSION")),
        blocker: None,
        repair_action: None,
        last_error: None,
        retry_after_ms: None,
        recovery_attempts,
        updated_at_unix: now_unix(),
    }
}

pub(super) fn remote_dependency_report() -> Value {
    json!([
        {
            "name": "remote_posix_shell",
            "classification": "required",
            "state": "checked_during_peer_ensure",
            "message": "Linux/macOS remote peer install uses a POSIX shell for service setup"
        },
        {
            "name": "remote_systemd_user",
            "classification": "optional",
            "state": "preferred_on_linux",
            "message": "user systemd is the preferred Linux remote peer service manager"
        },
        {
            "name": "remote_nohup_supervisor",
            "classification": "optional",
            "state": "fallback_on_linux",
            "message": "managed nohup supervisor is used when user systemd is unavailable"
        },
        {
            "name": "remote_launchd_user",
            "classification": "optional",
            "state": "macos_provider",
            "message": "LaunchAgent provider is used for macOS remotes"
        },
        {
            "name": "remote_schtasks_user",
            "classification": "optional",
            "state": "windows_provider",
            "message": "scheduled task provider is used for Windows user-scope remotes"
        }
    ])
}

pub(super) fn remote_peer_blocker(error: &str) -> String {
    if error.contains("SSH authentication failed") {
        "ssh_auth_failed".to_string()
    } else if error.contains("ProxyCommand")
        || error.contains("unsupported --ssh-arg")
        || error.contains("unsupported -o")
    {
        "ssh_config_unsupported".to_string()
    } else {
        "remote_peer_install_failed".to_string()
    }
}

pub(super) fn remote_transport_protocols(result: &deploy::RemoteInstallResult) -> Vec<String> {
    let mut protocols = Vec::new();
    if result.remote_quic_transport.is_some() {
        protocols.push("quic".to_string());
    }
    if result.remote_tls_transport.is_some() {
        protocols.push("tls-tcp".to_string());
    }
    protocols.push("plain-tcp".to_string());
    protocols
}

fn value_hash(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.to_string().as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

pub(super) fn remote_peer_lifecycle_report(
    alias: &str,
    phase: peer_lifecycle::workflow::PeerLifecyclePhase,
    operation: peer_lifecycle::workflow::LifecycleOperation,
    install_args: Option<&cli::InstallRemoteArgs>,
    remote_path: Option<&str>,
    service_manager: &str,
    blocker: Option<&str>,
    last_error: Option<&str>,
    recovery_attempts: u32,
) -> Value {
    remote_peer_lifecycle_report_record(
        alias,
        phase,
        operation,
        install_args,
        remote_path,
        service_manager,
        blocker,
        last_error,
        recovery_attempts,
    )
    .to_redacted_value()
}

pub(super) fn remote_peer_lifecycle_report_record(
    alias: &str,
    phase: peer_lifecycle::workflow::PeerLifecyclePhase,
    operation: peer_lifecycle::workflow::LifecycleOperation,
    install_args: Option<&cli::InstallRemoteArgs>,
    remote_path: Option<&str>,
    service_manager: &str,
    blocker: Option<&str>,
    last_error: Option<&str>,
    recovery_attempts: u32,
) -> peer_lifecycle::report::PeerLifecycleReport {
    let mut report = if let Some(args) = install_args {
        let provider = peer_lifecycle::service_provider::provider_for_remote_report(
            service_manager,
            args.remote_os.into(),
            args.persist.into(),
        );
        let remote_path = remote_path
            .or(args.remote_path.as_deref())
            .unwrap_or("ssh_proxy");
        let spec = peer_lifecycle::spec::PeerLifecycleSpec::remote_peer(
            alias.to_string(),
            remote_path,
            args,
            provider,
        );
        peer_lifecycle::workflow::phase_report_for_operation(&spec, operation, phase)
    } else {
        let mut report = peer_lifecycle::report::PeerLifecycleReport::new(alias, phase);
        report.operation = Some(operation.as_str().to_string());
        if let Some(provider) =
            peer_lifecycle::service_provider::ServiceProviderKind::from_manager_name(
                service_manager,
            )
        {
            report.provider = Some(provider.manager_name().to_string());
        }
        report
    };
    report.service_manager = Some(service_manager.to_string());
    report.recovery_attempts = recovery_attempts;
    report.blocker = blocker.map(ToOwned::to_owned);
    report.last_error = last_error.map(ToOwned::to_owned);
    report
}

pub(super) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
