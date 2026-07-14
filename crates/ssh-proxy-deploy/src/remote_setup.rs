use std::collections::BTreeMap;

use serde_json::{Value, json};
use ssh_proxy_core::external::{ExternalActionClass, ExternalActionReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteArtifactKind {
    VscodeMachineSettings,
    VscodeServerEnv,
    VscodeRemoteStatus,
}

impl RemoteArtifactKind {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::VscodeMachineSettings => "settings.json",
            Self::VscodeServerEnv => "server-env-setup",
            Self::VscodeRemoteStatus => "remote-proxy-status.json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteArtifactIntent {
    pub server_dir: String,
    pub relative_path: String,
    pub artifact: RemoteArtifactKind,
    pub backup_existing: bool,
    pub label: String,
}

impl RemoteArtifactIntent {
    pub fn new(
        server_dir: impl Into<String>,
        relative_path: impl Into<String>,
        artifact: RemoteArtifactKind,
        backup_existing: bool,
        label: impl Into<String>,
    ) -> Self {
        Self {
            server_dir: server_dir.into(),
            relative_path: relative_path.into(),
            artifact,
            backup_existing,
            label: label.into(),
        }
    }

    pub fn read_command(&self) -> String {
        build_remote_setup_read_command(&self.server_dir, &self.relative_path)
    }

    pub fn write_command(&self) -> String {
        build_remote_setup_write_command(
            &self.server_dir,
            &self.relative_path,
            self.backup_existing,
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteSetupPlan {
    pub artifacts: Vec<RemoteArtifactIntent>,
}

impl RemoteSetupPlan {
    pub fn new(artifacts: Vec<RemoteArtifactIntent>) -> Self {
        Self { artifacts }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSetupPayloadInput {
    pub target: String,
    pub workspace_id: String,
    pub workspace_paths: Vec<String>,
    pub remote_url: String,
    pub bind_host: String,
    pub port: u16,
    pub connect_mode: String,
    pub route_id: String,
    pub job_id: String,
    pub route_owner: Option<String>,
    pub selected_transport: Option<String>,
    pub fallback_reason: Option<String>,
    pub local_proxy: String,
    pub server_dir: String,
    pub no_proxy: String,
    pub proxy_support: String,
    pub terminal_env: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSetupScriptIntent {
    pub command: String,
    pub label: String,
    pub class: ExternalActionClass,
}

impl RemoteSetupScriptIntent {
    pub fn new(
        command: impl Into<String>,
        label: impl Into<String>,
        class: ExternalActionClass,
    ) -> Self {
        Self {
            command: command.into(),
            label: label.into(),
            class,
        }
    }

    pub fn fallback_shell(label: impl Into<String>) -> Self {
        Self::new("sh -s", label, ExternalActionClass::FallbackProvider)
    }

    pub fn external_action_report(&self) -> ExternalActionReport {
        ExternalActionReport::new(self.class, "remote_shell_bootstrap", true)
            .with_reason(format!("{} via fallback shell script", self.label))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSetupExecutionPlan {
    pub payload: Value,
    pub artifacts: Vec<RemoteArtifactIntent>,
    pub scripts: Vec<RemoteSetupScriptIntent>,
}

impl RemoteSetupExecutionPlan {
    pub fn new(payload: Value) -> Self {
        Self {
            payload,
            artifacts: Vec::new(),
            scripts: Vec::new(),
        }
    }

    pub fn with_artifact(mut self, artifact: RemoteArtifactIntent) -> Self {
        self.artifacts.push(artifact);
        self
    }

    pub fn with_script(mut self, script: RemoteSetupScriptIntent) -> Self {
        self.scripts.push(script);
        self
    }
}

pub fn build_remote_setup_payload(input: RemoteSetupPayloadInput) -> Value {
    let env = build_proxy_env(&input.remote_url, &input.no_proxy);
    let mut values = serde_json::Map::new();
    values.insert("http.proxy".to_string(), json!(&input.remote_url));
    values.insert("http.proxySupport".to_string(), json!(&input.proxy_support));
    if input.terminal_env {
        values.insert(
            "terminal.integrated.env.linux".to_string(),
            json!(env.clone()),
        );
        values.insert(
            "terminal.integrated.env.osx".to_string(),
            json!(env.clone()),
        );
        values.insert("terminal.integrated.env.windows".to_string(), json!(env));
    }
    json!({
        "target": input.target,
        "workspaceId": input.workspace_id,
        "workspacePaths": input.workspace_paths,
        "proxyUrl": input.remote_url,
        "bindHost": input.bind_host,
        "port": input.port,
        "connectMode": input.connect_mode,
        "routeId": input.route_id,
        "jobId": input.job_id,
        "routeOwner": input.route_owner,
        "selectedTransport": input.selected_transport,
        "fallbackReason": input.fallback_reason,
        "localProxySource": "daemon",
        "localProxyUrl": input.local_proxy,
        "backend": "ssh_proxy",
        "server_dir": input.server_dir,
        "no_proxy": input.no_proxy,
        "proxy_support": input.proxy_support,
        "values": values,
    })
}

pub fn build_proxy_env(proxy_url: &str, no_proxy: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("HTTP_PROXY".to_string(), proxy_url.to_string());
    env.insert("HTTPS_PROXY".to_string(), proxy_url.to_string());
    env.insert("ALL_PROXY".to_string(), proxy_url.to_string());
    env.insert("NO_PROXY".to_string(), no_proxy.to_string());
    env.insert("http_proxy".to_string(), proxy_url.to_string());
    env.insert("https_proxy".to_string(), proxy_url.to_string());
    env.insert("all_proxy".to_string(), proxy_url.to_string());
    env.insert("no_proxy".to_string(), no_proxy.to_string());
    env
}

fn build_remote_setup_read_command(server_dir: &str, relative_path: &str) -> String {
    format!(
        "set -eu; server_dir={server_dir}; relative_path={relative_path}; target=\"$HOME/$server_dir/$relative_path\"; if [ -f \"$target\" ]; then cat \"$target\"; fi",
        server_dir = shell_quote(server_dir),
        relative_path = shell_quote(relative_path),
    )
}

fn build_remote_setup_write_command(
    server_dir: &str,
    relative_path: &str,
    backup_existing: bool,
) -> String {
    let backup = if backup_existing {
        "if [ -f \"$target\" ]; then cp \"$target\" \"$target.vscode-remote-proxy.bak\" 2>/dev/null || true; fi; "
    } else {
        ""
    };
    format!(
        "set -eu; server_dir={server_dir}; relative_path={relative_path}; target=\"$HOME/$server_dir/$relative_path\"; mkdir -p \"$(dirname \"$target\")\"; tmp=\"$target.tmp.$$\"; umask 077; cat > \"$tmp\"; {backup}mv \"$tmp\" \"$target\"; chmod 600 \"$target\" 2>/dev/null || true",
        server_dir = shell_quote(server_dir),
        relative_path = shell_quote(relative_path),
    )
}

fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_artifact_intent_renders_stdin_write_command() {
        let intent = RemoteArtifactIntent::new(
            ".vscode-server",
            "data/Machine/settings.json",
            RemoteArtifactKind::VscodeMachineSettings,
            true,
            "write settings",
        );

        let command = intent.write_command();

        assert!(command.contains("cat > \"$tmp\""));
        assert!(command.contains(".vscode-server"));
        assert!(command.contains("settings.json"));
        assert!(!command.contains("http.proxy"));
        assert!(!command.contains("<<"));
    }

    #[test]
    fn remote_setup_plan_groups_artifact_intents() {
        let plan = RemoteSetupPlan::new(vec![RemoteArtifactIntent::new(
            ".vscode-server",
            "remote-proxy-status.json",
            RemoteArtifactKind::VscodeRemoteStatus,
            false,
            "write status",
        )]);

        assert_eq!(plan.artifacts.len(), 1);
        assert_eq!(
            plan.artifacts[0].artifact.file_name(),
            "remote-proxy-status.json"
        );
    }

    #[test]
    fn remote_setup_payload_preserves_vscode_fields() {
        let payload = build_remote_setup_payload(RemoteSetupPayloadInput {
            target: "box".to_string(),
            workspace_id: "workspace".to_string(),
            workspace_paths: vec!["/repo".to_string()],
            remote_url: "http://127.0.0.1:18080".to_string(),
            bind_host: "127.0.0.1".to_string(),
            port: 18080,
            connect_mode: "reverse-link".to_string(),
            route_id: "route-a".to_string(),
            job_id: "job-a".to_string(),
            route_owner: Some("daemon".to_string()),
            selected_transport: Some("ssh-native".to_string()),
            fallback_reason: None,
            local_proxy: "socks5h://127.0.0.1:1080".to_string(),
            server_dir: ".vscode-server".to_string(),
            no_proxy: "localhost,127.0.0.1".to_string(),
            proxy_support: "override".to_string(),
            terminal_env: true,
        });

        assert_eq!(payload["target"], "box");
        assert_eq!(payload["values"]["http.proxy"], "http://127.0.0.1:18080");
        assert_eq!(
            payload["values"]["terminal.integrated.env.linux"]["HTTP_PROXY"],
            "http://127.0.0.1:18080"
        );
    }

    #[test]
    fn fallback_shell_script_reports_external_action() {
        let intent = RemoteSetupScriptIntent::fallback_shell("write vscode settings");
        let value = intent.external_action_report().to_json();

        assert_eq!(value["class"], "fallback_provider");
        assert_eq!(value["execution_backend"], "remote_shell_bootstrap");
        assert_eq!(value["fallback_used"], true);
    }
}
