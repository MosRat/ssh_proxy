use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::external::ExternalActionClass;

#[cfg(windows)]
pub use windows_service;
#[cfg(windows)]
pub use windows_sys;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCommandPlan {
    pub program: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub class: ExternalActionClass,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformProbePlan {
    pub command: PlatformCommandPlan,
    pub expected_state: String,
}

impl PlatformCommandPlan {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        class: ExternalActionClass,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            class,
            reason: reason.into(),
            repair_action: None,
        }
    }

    pub fn with_repair_action(mut self, repair_action: impl Into<String>) -> Self {
        self.repair_action = Some(repair_action.into());
        self
    }

    pub fn command_line(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl PlatformProbePlan {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        class: ExternalActionClass,
        reason: impl Into<String>,
        expected_state: impl Into<String>,
    ) -> Self {
        Self {
            command: PlatformCommandPlan::new(program, args, class, reason),
            expected_state: expected_state.into(),
        }
    }

    pub fn with_repair_action(mut self, repair_action: impl Into<String>) -> Self {
        self.command = self.command.with_repair_action(repair_action);
        self
    }

    pub fn command_plan(&self) -> &PlatformCommandPlan {
        &self.command
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformCommandOutcome {
    pub plan: PlatformCommandPlan,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformScriptPlan {
    pub command: PlatformCommandPlan,
    pub script_path: String,
}

impl PlatformScriptPlan {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        script_path: impl Into<String>,
        class: ExternalActionClass,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            command: PlatformCommandPlan::new(program, args, class, reason),
            script_path: script_path.into(),
        }
    }

    pub fn with_repair_action(mut self, repair_action: impl Into<String>) -> Self {
        self.command = self.command.with_repair_action(repair_action);
        self
    }

    pub fn command_plan(&self) -> &PlatformCommandPlan {
        &self.command
    }
}

impl PlatformCommandOutcome {
    pub fn to_json(&self) -> Value {
        json!({
            "program": self.plan.program,
            "args": self.plan.args,
            "class": self.plan.class.as_str(),
            "reason": self.plan.reason,
            "repair_action": self.plan.repair_action,
            "ok": self.ok,
            "status_code": self.status_code,
            "stdout": self.stdout,
            "stderr": self.stderr,
        })
    }
}

pub fn capture_command(plan: PlatformCommandPlan) -> Result<PlatformCommandOutcome> {
    let output = Command::new(&plan.program)
        .args(&plan.args)
        .output()
        .with_context(|| format!("failed to run {}", plan.command_line()))?;
    Ok(PlatformCommandOutcome {
        plan,
        ok: output.status.success(),
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn spawn_command(plan: PlatformCommandPlan) -> Result<()> {
    Command::new(&plan.program)
        .args(&plan.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {}", plan.command_line()))?;
    Ok(())
}

pub fn command_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .map(|output| output.status.success() || output.status.code().is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_plan_renders_classification() {
        let plan = PlatformCommandPlan::new(
            "systemctl",
            ["--user", "status", "ssh-proxy-helper.service"],
            ExternalActionClass::RequiredProvider,
            "query local user systemd service",
        )
        .with_repair_action("rerun daemon install");
        let outcome = PlatformCommandOutcome {
            plan,
            ok: false,
            status_code: Some(3),
            stdout: String::new(),
            stderr: "inactive".to_string(),
        };
        let value = outcome.to_json();

        assert_eq!(value["class"], "required_provider");
        assert_eq!(value["repair_action"], "rerun daemon install");
        assert_eq!(value["status_code"], 3);
    }

    #[test]
    fn script_plan_carries_self_update_classification() {
        let plan = PlatformScriptPlan::new(
            "sh",
            ["switch.sh"],
            "switch.sh",
            ExternalActionClass::SelfUpdate,
            "launch daemon self-update switch script",
        );

        assert_eq!(plan.command_plan().class, ExternalActionClass::SelfUpdate);
        assert_eq!(plan.script_path, "switch.sh");
    }

    #[test]
    fn probe_plan_carries_external_classification() {
        let plan = PlatformProbePlan::new(
            "systemctl",
            ["--user", "status", "ssh_proxy.service"],
            ExternalActionClass::RequiredProvider,
            "probe local systemd service",
            "service should be queryable",
        )
        .with_repair_action("rerun daemon install");

        assert_eq!(
            plan.command_plan().class,
            ExternalActionClass::RequiredProvider
        );
        assert_eq!(plan.expected_state, "service should be queryable");
        assert_eq!(
            plan.command_plan().repair_action.as_deref(),
            Some("rerun daemon install")
        );
    }
}
