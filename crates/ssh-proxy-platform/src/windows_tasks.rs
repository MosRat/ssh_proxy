use anyhow::{Result, anyhow};
use planif::{
    enums::TaskCreationFlags,
    schedule::TaskScheduler,
    schedule_builder::{Action, ScheduleBuilder},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use ssh_proxy_core::external::ExternalActionClass;

use crate::{ExecutionBackend, NativeProviderOutcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsScheduledTaskPlan {
    pub task_name: String,
    pub program: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub description: String,
}

impl WindowsScheduledTaskPlan {
    pub fn new(
        task_name: impl Into<String>,
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            task_name: task_name.into(),
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            description: "ssh_proxy user daemon".to_string(),
        }
    }

    pub fn action_arguments(&self) -> String {
        self.args
            .iter()
            .map(|arg| windows_action_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub fn register_logon_task(plan: &WindowsScheduledTaskPlan) -> Result<NativeProviderOutcome> {
    let scheduler = planif_result(
        TaskScheduler::new(),
        "failed to initialize Task Scheduler COM",
    )?;
    let com = scheduler.get_com();
    let builder = planif_result(
        ScheduleBuilder::new(&com),
        "failed to create Task Scheduler builder",
    )?
    .create_logon()
    .author("ssh_proxy")
    .map_err(|err| anyhow!("failed to set scheduled task author: {err}"))?
    .description(&plan.description)
    .map_err(|err| anyhow!("failed to set scheduled task description: {err}"))?
    .trigger("ssh_proxy_logon", true)
    .map_err(|err| anyhow!("failed to create logon trigger: {err}"))?
    .action(Action::new(
        "ssh_proxy_daemon",
        &plan.program,
        "",
        &plan.action_arguments(),
    ))
    .map_err(|err| anyhow!("failed to set scheduled task action: {err}"))?
    .user_id("")
    .map_err(|err| anyhow!("failed to set scheduled task logon user: {err}"))?;
    planif_result(builder.build(), "failed to build scheduled task")?
        .register(&plan.task_name, TaskCreationFlags::CreateOrUpdate as i32)
        .map_err(|err| anyhow!("failed to register scheduled task: {err}"))?;
    Ok(NativeProviderOutcome::new(
        true,
        ExecutionBackend::Com,
        ExternalActionClass::RequiredProvider,
        "register Windows user daemon task through Task Scheduler COM",
    )
    .with_status("registered")
    .with_details(json!({
        "task_name": plan.task_name,
        "program": plan.program,
        "args": plan.args,
    })))
}

fn planif_result<T>(
    result: std::result::Result<T, Box<dyn std::error::Error>>,
    context: &str,
) -> Result<T> {
    result.map_err(|err| anyhow!("{context}: {err}"))
}

fn windows_action_arg(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if !value.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return value.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_action_arguments_quote_spaces() {
        let plan = WindowsScheduledTaskPlan::new(
            "ssh_proxy",
            r"C:\Program Files\ssh_proxy\ssh_proxy.exe",
            [
                "daemon",
                "serve",
                "--control",
                r"npipe:////./pipe/ssh proxy",
            ],
        );

        assert!(
            plan.action_arguments()
                .contains("\"npipe:////./pipe/ssh proxy\"")
        );
        assert!(!plan.action_arguments().contains("schtasks"));
    }
}
