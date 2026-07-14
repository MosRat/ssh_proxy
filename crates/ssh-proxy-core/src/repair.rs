use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepairAction {
    pub id: String,
    pub label: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub interactive: bool,
    pub requires_elevation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl RepairAction {
    fn new(id: &str, label: &str, kind: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            command: None,
            interactive: false,
            requires_elevation: false,
            retry_after_ms: None,
            message: None,
        }
    }

    fn command(mut self, command: &str) -> Self {
        self.command = Some(command.to_string());
        self
    }

    fn interactive(mut self, requires_elevation: bool) -> Self {
        self.interactive = true;
        self.requires_elevation = requires_elevation;
        self
    }

    fn retry(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    fn message(mut self, message: &str) -> Self {
        self.message = Some(message.to_string());
        self
    }
}

pub fn action_for_blocker(blocker: &str) -> Option<RepairAction> {
    match blocker {
        "daemon_unavailable" => Some(
            RepairAction::new("install_daemon", "Install ssh_proxy daemon", "daemon_install")
                .command("ssh_proxy daemon install --scope system --elevate")
                .interactive(true)
                .retry(1000)
                .message("Install or start the system daemon, then retry the proxy session."),
        ),
        "daemon_pipe_access_denied" => Some(
            RepairAction::new("repair_daemon_acl", "Repair daemon permissions", "daemon_reinstall")
                .command("ssh_proxy daemon install --scope system --elevate")
                .interactive(true)
                .retry(1000)
                .message("Reinstall the daemon to repair the private control endpoint ACL."),
        ),
        "node_control_token_required" | "invalid_node_control_token" => Some(
            RepairAction::new(
                "reinstall_stale_daemon",
                "Repair stale daemon configuration",
                "daemon_reinstall",
            )
            .command("ssh_proxy daemon install --scope system --elevate")
            .interactive(true)
            .message("The running daemon is using stale token-based control; reinstall to migrate it."),
        ),
        "requires_elevation" => Some(
            RepairAction::new("run_elevated_install", "Run elevated install", "daemon_install")
                .command("ssh_proxy daemon install --scope system --elevate")
                .interactive(true)
                .message("This action needs an interactive elevation prompt."),
        ),
        "cancelled_by_user" => Some(
            RepairAction::new("retry_daemon_install", "Retry daemon install", "daemon_install")
                .command("ssh_proxy daemon install --scope system --elevate")
                .interactive(true)
                .message("The elevated install was cancelled before the daemon could be repaired."),
        ),
        "route_already_running_different_spec" => Some(
            RepairAction::new("restart_conflicting_route", "Restart conflicting route", "route_repair")
                .command("ssh_proxy down --target <target> --json")
                .message("Stop or adopt the existing daemon-owned route, then retry."),
        ),
        "handoff_timeout" | "remote_port_refused" | "remote_port_not_ready" => Some(
            RepairAction::new("retry_remote_handoff", "Retry remote handoff", "proxy_session_retry")
                .command("ssh_proxy doctor --json --report")
                .retry(1000)
                .message("The daemon will retry once; collect a report if the remote port stays unavailable."),
        ),
        "ssh_auth_failed" => Some(
            RepairAction::new("repair_ssh_auth", "Repair SSH authentication", "ssh_auth")
                .command("ssh-add <identity-file>")
                .message("Make the same identity available to the Windows OpenSSH agent/Pageant or use an unencrypted identity file."),
        ),
        "ssh_config_unsupported" => Some(
            RepairAction::new(
                "use_emergency_external_ssh",
                "Use explicit external SSH compatibility",
                "emergency_compat",
            )
            .message("The Rust SSH path found an unsupported OpenSSH directive; external SSH must be explicitly allowed."),
        ),
        "remote_setup_failed" => Some(
            RepairAction::new("rerun_apply_settings", "Apply remote settings again", "remote_setup")
                .command("ssh_proxy vscode apply-settings --json")
                .message("Rerun daemon-owned remote setup after checking the report."),
        ),
        "remote_peer_install_failed" => Some(
            RepairAction::new("repair_remote_peer", "Repair remote peer", "remote_peer_repair")
                .command("ssh_proxy doctor --json --report --target <target>")
                .retry(1000)
                .message("Collect a daemon report, then retry the daemon-owned remote peer install."),
        ),
        _ => None,
    }
}

pub fn action_value_for_blocker(blocker: &str) -> Value {
    action_for_blocker(blocker)
        .and_then(|action| serde_json::to_value(action).ok())
        .unwrap_or(Value::Null)
}

pub fn attach_repair_action(object: &mut serde_json::Map<String, Value>, blocker: &str) {
    if let Some(action) = action_for_blocker(blocker) {
        object.insert("repair_action".to_string(), json!(action));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_token_maps_to_elevated_reinstall() {
        let action = action_for_blocker("node_control_token_required").unwrap();
        assert_eq!(action.kind, "daemon_reinstall");
        assert!(action.requires_elevation);
        assert_eq!(
            action.command.as_deref(),
            Some("ssh_proxy daemon install --scope system --elevate")
        );
    }

    #[test]
    fn unsupported_ssh_config_is_emergency_compat() {
        let action = action_for_blocker("ssh_config_unsupported").unwrap();
        assert_eq!(action.kind, "emergency_compat");
        assert!(!action.requires_elevation);
    }
}
