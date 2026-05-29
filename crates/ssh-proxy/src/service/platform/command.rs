use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub(super) fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to run {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{program} exited with status {status}")
    }
}

#[allow(dead_code)]
pub(super) fn run_command_output(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(())
    } else {
        bail!("{program} exited with status {}", output.status)
    }
}

pub(super) fn capture_command_output(program: &str, args: &[&str]) -> Value {
    match Command::new(program).args(args).output() {
        Ok(output) => json!({
            "ok": output.status.success(),
            "program": program,
            "args": args,
            "status": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
        }),
        Err(err) => json!({
            "ok": false,
            "program": program,
            "args": args,
            "error": err.to_string(),
        }),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn write_text(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))
}
