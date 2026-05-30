use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

pub(super) fn openssh_command(target: &str, accept_new: bool, remote_command: &str) -> Command {
    openssh_command_for_target(target, accept_new, &[remote_command])
}

pub(super) fn openssh_command_for_target(
    target: &str,
    accept_new: bool,
    remote_commands: &[&str],
) -> Command {
    let remote_script = remote_commands.join("; ");
    let mut command = Command::new("ssh");
    command
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg(if accept_new {
            "StrictHostKeyChecking=accept-new"
        } else {
            "StrictHostKeyChecking=yes"
        })
        .arg(target)
        .arg(format!("sh -lc {}", sh_quote(&remote_script)));
    command
}

pub(super) fn scp_command(
    sidecar: &Path,
    target: &str,
    accept_new: bool,
    remote_path: &str,
) -> Command {
    let mut command = Command::new("scp");
    command
        .arg("-q")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg(if accept_new {
            "StrictHostKeyChecking=accept-new"
        } else {
            "StrictHostKeyChecking=yes"
        })
        .arg(sidecar)
        .arg(format!("{target}:{remote_path}"));
    command
}

pub(super) fn russh_host_exec_command(
    target: &str,
    accept_new: bool,
    upstream_proxy: Option<&str>,
    label: &str,
) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command.arg("--log").arg("warn").arg("host").arg(target);
    if accept_new {
        command.arg("--accept-new");
    }
    command
        .arg("exec")
        .arg("--stdin")
        .arg("--label")
        .arg(label)
        .arg("--timeout-secs")
        .arg("20")
        .arg("--json");
    if let Some(proxy) = upstream_proxy {
        command.env("HTTP_PROXY", proxy).env("HTTPS_PROXY", proxy);
    }
    command
}

pub(super) fn run_output(mut command: Command) -> Output {
    command
        .output()
        .unwrap_or_else(|err| panic!("failed to spawn remote e2e command: {err}"))
}

pub(super) fn run_with_stdin(mut command: Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn remote e2e command: {err}"));
    child
        .stdin
        .as_mut()
        .expect("remote e2e child stdin")
        .write_all(stdin.as_bytes())
        .expect("write remote e2e stdin");
    child.wait_with_output().expect("wait remote e2e command")
}

pub(super) fn assert_success(
    kind: &str,
    target: &str,
    topology: &str,
    output: &Output,
    message: &str,
) {
    assert!(
        output.status.success(),
        "{message}; kind={kind} target={target} topology={topology} classification={} status={} stderr={}",
        failure_class(output),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
}

pub(super) fn assert_stdout_contains(output: &Output, needle: &str, context: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let compact = stdout.replace(char::is_whitespace, "");
    assert!(
        compact.contains(needle),
        "{context} missing {needle}; stdout={stdout}"
    );
}

pub(super) fn failure_class(output: &Output) -> &'static str {
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    if stderr.contains("host key verification failed")
        || stderr.contains("strict host key checking")
    {
        "host_key"
    } else if stderr.contains("permission denied") || stderr.contains("publickey") {
        "auth"
    } else if stderr.contains("could not resolve hostname")
        || stderr.contains("name or service not known")
    {
        "name_resolution"
    } else if stderr.contains("connection timed out") || stderr.contains("operation timed out") {
        "network_timeout"
    } else if stderr.contains("connection refused") {
        "connection_refused"
    } else if stderr.contains("banner") {
        "banner"
    } else {
        "unknown"
    }
}

pub(super) fn json_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

pub(super) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
