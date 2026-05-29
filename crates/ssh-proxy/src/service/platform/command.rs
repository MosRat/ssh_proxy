use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use ssh_proxy_core::external::ExternalActionClass;
use ssh_proxy_platform::{PlatformCommandPlan, capture_command};

pub(super) fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let outcome = capture_command(command_plan(program, args))?;
    if outcome.ok {
        Ok(())
    } else {
        bail!(
            "{program} exited with status {:?}",
            outcome.status_code.unwrap_or_default()
        )
    }
}

#[allow(dead_code)]
pub(super) fn run_command_output(program: &str, args: &[&str]) -> Result<()> {
    let outcome = capture_command(command_plan(program, args))?;
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    if outcome.ok {
        Ok(())
    } else {
        bail!(
            "{program} exited with status {:?}",
            outcome.status_code.unwrap_or_default()
        )
    }
}

pub(super) fn capture_command_output(program: &str, args: &[&str]) -> Value {
    match capture_command(command_plan(program, args)) {
        Ok(outcome) => json!({
            "ok": outcome.ok,
            "program": outcome.plan.program,
            "args": outcome.plan.args,
            "class": outcome.plan.class.as_str(),
            "reason": outcome.plan.reason,
            "status": outcome.status_code,
            "stdout": outcome.stdout,
            "stderr": outcome.stderr,
        }),
        Err(err) => json!({
            "ok": false,
            "program": program,
            "args": args,
            "class": ExternalActionClass::RequiredProvider.as_str(),
            "error": err.to_string(),
        }),
    }
}

fn command_plan(program: &str, args: &[&str]) -> PlatformCommandPlan {
    PlatformCommandPlan::new(
        program,
        args.iter().copied(),
        ExternalActionClass::RequiredProvider,
        "run local service provider command",
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn write_text(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}
