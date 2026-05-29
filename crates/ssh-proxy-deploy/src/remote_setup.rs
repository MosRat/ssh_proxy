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
}
