use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) struct ChildGuard {
    child: Option<Child>,
}

#[derive(Debug)]
pub(super) struct TcpMeasurement {
    pub(super) response: String,
    pub(super) bytes: u64,
    pub(super) duration_ms: u128,
    pub(super) first_byte_ms: u128,
}

impl ChildGuard {
    pub(super) fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    pub(super) fn kill_and_wait(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
    }

    pub(super) fn has_exited(&mut self) -> bool {
        self.child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
            .is_some()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

pub(super) fn tool_available(name: &str) -> bool {
    let mut command = Command::new(name);
    match name {
        "ssh" | "scp" => {
            command.arg("-V");
        }
        "curl" => {
            command.arg("--version");
        }
        _ => {}
    }
    command.output().is_ok()
}

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
    local_path: &Path,
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
        .arg(local_path)
        .arg(format!("{target}:{remote_path}"));
    command
}

pub(super) fn russh_host_exec_command(
    local_bin: &Path,
    target: &str,
    accept_new: bool,
    label: &str,
) -> Command {
    let mut command = Command::new(local_bin);
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
    command
}

pub(super) fn run_output(mut command: Command) -> Result<Output, String> {
    command
        .output()
        .map_err(|err| format!("failed to spawn command: {err}"))
}

pub(super) fn run_with_stdin(mut command: Command, stdin: &str) -> Result<Output, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn command: {err}"))?;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| "failed to open child stdin".to_string())?
        .write_all(stdin.as_bytes())
        .map_err(|err| format!("failed to write child stdin: {err}"))?;
    child
        .wait_with_output()
        .map_err(|err| format!("failed to wait for child: {err}"))
}

pub(super) fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral port");
    listener.local_addr().expect("read local addr")
}

pub(super) fn wait_tcp(addr: SocketAddr, child: &mut ChildGuard) -> Result<(), String> {
    for _ in 0..60 {
        if TcpStream::connect(addr).is_ok() {
            return Ok(());
        }
        if child.has_exited() {
            return Err(format!("proxy process exited before {addr} became ready"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{addr} did not become ready"))
}

pub(super) fn control_status_via_tcp(
    proxy: SocketAddr,
    token: &str,
) -> Result<TcpMeasurement, String> {
    let started = Instant::now();
    let mut stream = TcpStream::connect(proxy).map_err(|err| format!("connect proxy: {err}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|err| format!("set read timeout: {err}"))?;
    let request = format!(r#"{{"cmd":"status","auth_token":{}}}"#, json_string(token));
    stream
        .write_all(format!("{request}\n").as_bytes())
        .map_err(|err| format!("write control request: {err}"))?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|err| format!("shutdown write: {err}"))?;

    let mut response = Vec::new();
    let mut first_byte_ms = None;
    let mut buf = [0_u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if first_byte_ms.is_none() {
                    first_byte_ms = Some(started.elapsed().as_millis());
                }
                response.extend_from_slice(&buf[..n]);
            }
            Err(err) => return Err(format!("read control response: {err}")),
        }
    }

    let duration_ms = started.elapsed().as_millis();
    let response = String::from_utf8(response).map_err(|err| format!("utf8 response: {err}"))?;
    Ok(TcpMeasurement {
        bytes: response.len() as u64,
        response,
        duration_ms,
        first_byte_ms: first_byte_ms.unwrap_or(duration_ms),
    })
}

pub(super) fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ssh_proxy-{name}-{}", stamp()));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

pub(super) fn temp_path(name: &str, ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ssh_proxy-{name}-{}.{}", stamp(), ext))
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
    } else if stderr.contains("certificate") || stderr.contains("cert") {
        "cert"
    } else if stderr.contains("protocol") || stderr.contains("handshake") {
        "protocol"
    } else if stderr.contains("connection closed") || stderr.contains("broken pipe") {
        "transient_network"
    } else {
        "unknown"
    }
}

pub(super) fn output_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        stderr
    }
}

pub(super) fn direct_host_from_ssh_config(target: &str) -> String {
    let output = Command::new("ssh").arg("-G").arg(target).output();
    let Ok(output) = output else {
        return target.to_string();
    };
    if !output.status.success() {
        return target.to_string();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            (key.eq_ignore_ascii_case("hostname")).then(|| parts.next().map(str::to_string))?
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| target.to_string())
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

fn stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("{nanos}-{}", std::process::id())
}
