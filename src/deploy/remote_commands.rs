use std::net::SocketAddr;

use anyhow::Result;
use serde_json::Value;

use crate::{cli, ssh_client};

pub(super) fn remote_status_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let version_check = format!("{} --version 2>/dev/null || true", sh_quote(remote_path));
    format!(
        "set +e; printf 'remote_binary_version='; {version_check}; \
             printf 'systemd_user_status=\\n'; systemctl --user status --no-pager ssh-proxy-helper.service 2>&1 || true; \
             printf '\\nnohup_status=\\n'; {status}",
        status = remote_nohup_status_snippet(remote_tcp)
    )
}

pub(super) fn remote_node_control_command(
    remote_path: &str,
    remote_control: SocketAddr,
    remote_token: Option<&str>,
    control_args: &str,
) -> String {
    let token_arg = remote_token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default();
    format!(
        "set -eu; {} node control --endpoint tcp://{}{} {}",
        sh_quote(remote_path),
        remote_control,
        token_arg,
        control_args
    )
}

pub(super) fn remote_node_control_json_command(
    remote_path: &str,
    remote_control: SocketAddr,
    remote_token: Option<&str>,
    request: &Value,
) -> String {
    let request = request.to_string();
    remote_node_control_command(
        remote_path,
        remote_control,
        remote_token,
        &format!("send {}", sh_quote(&request)),
    )
}

pub(super) async fn default_persistent_remote_path(
    client: &ssh_client::Client,
    remote_os: cli::RemoteOs,
) -> Result<String> {
    if remote_os == cli::RemoteOs::Windows {
        return Ok(r"%LOCALAPPDATA%\ssh_proxy\bin\ssh_proxy.exe".to_string());
    }
    let command = format!(
        "set -eu; home=\"${{HOME:-/tmp}}\"; base=\"\"; for d in \"$home/.local/bin\" \"$home/bin\" \"$home/.ssh_proxy/bin\"; do case \":${{PATH:-}}:\" in *\":$d:\"*) base=\"$d\"; break;; esac; done; if [ -z \"$base\" ]; then base=\"$home/.local/bin\"; fi; mkdir -p \"$base\"; case \":${{PATH:-}}:\" in *\":$base:\"*) ;; *) profile=\"$home/.profile\"; touch \"$profile\" 2>/dev/null && if ! grep -q 'ssh_proxy managed PATH' \"$profile\" 2>/dev/null; then printf '\\n# ssh_proxy managed PATH\\ncase \":$PATH:\" in *\":$HOME/.local/bin:\"*) ;; *) export PATH=\"$HOME/.local/bin:$PATH\";; esac\\n' >> \"$profile\"; fi || true;; esac; printf '%s' \"$base/ssh_proxy\""
    );
    let output = client.exec_output(command).await?;
    Ok(output.trim().to_string())
}

pub(super) fn remote_auto_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    format!(
        "set -eu; if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then {systemd}; else {nohup}; fi",
        systemd = remote_systemd_install_command(remote_path, args),
        nohup = remote_nohup_start_command(remote_path, args, true)
    )
}

pub(super) fn remote_write_config_command(
    preferred_transport: SocketAddr,
    preferred_control: SocketAddr,
    token: &str,
    local_node_id: Option<&str>,
    local_node_name: Option<&str>,
    local_control_endpoint: Option<&str>,
    local_transport: Option<SocketAddr>,
) -> String {
    let peer_table = local_node_id
            .map(|node_id| {
                let node_name = toml_quote(local_node_name.unwrap_or("local"));
                let control = local_control_endpoint
                    .map(toml_quote)
                    .map(|value| format!("control_endpoint = {value}\n"))
                    .unwrap_or_default();
                let transport = local_transport
                    .map(|addr| toml_quote(&addr.to_string()))
                    .map(|value| format!("transport = {value}\n"))
                    .unwrap_or_default();
                format!(
                    "\n[peers.bootstrap-local]\nnode_id = {node_id}\nnode_name = {node_name}\ntrust = \"ssh-bootstrap\"\n{control}{transport}",
                    node_id = toml_quote(node_id),
                )
            })
            .unwrap_or_default();
    format!(
        r#"set -eu
mkdir -p "$HOME/.ssh_proxy"
is_free() {{
  port="$1"
  if command -v ss >/dev/null 2>&1; then
    ! ss -ltn 2>/dev/null | grep -Eq "[.:]${{port}}[[:space:]]"
  elif command -v netstat >/dev/null 2>&1; then
    ! netstat -ltn 2>/dev/null | grep -Eq "[.:]${{port}}[[:space:]]"
  else
    return 0
  fi
}}
pick_port() {{
  start="$1"
  end=$((start + 199))
  port="$start"
  while [ "$port" -le "$end" ]; do
    if is_free "$port"; then printf '%s' "$port"; return 0; fi
    port=$((port + 1))
  done
  printf '%s' "$start"
}}
transport_port=$(pick_port {transport_port})
control_port=$(pick_port {control_port})
transport="127.0.0.1:$transport_port"
control="127.0.0.1:$control_port"
config_file="$HOME/.ssh_proxy/config.toml"
existing_node_id=""
existing_node_name=""
if [ -f "$config_file" ]; then
  existing_node_id="$(sed -n 's/^node_id = "\(.*\)"$/\1/p' "$config_file" 2>/dev/null | head -n 1 || true)"
  existing_node_name="$(sed -n 's/^node_name = "\(.*\)"$/\1/p' "$config_file" 2>/dev/null | head -n 1 || true)"
fi
node_name="$(id -un 2>/dev/null || printf unknown)@$(hostname 2>/dev/null || printf unknown)"
node_id=""
if command -v od >/dev/null 2>&1; then
  node_id="spx-$(od -An -N32 -tx1 /dev/urandom 2>/dev/null | tr -d ' \n')"
fi
if [ -z "$node_id" ]; then
  node_id="spx-$(date +%s)-$$"
fi
if [ -n "$existing_node_id" ]; then node_id="$existing_node_id"; fi
if [ -n "$existing_node_name" ]; then node_name="$existing_node_name"; fi
created_at_unix="$(date +%s 2>/dev/null || printf 0)"
if [ -f "$config_file" ]; then
  cp "$config_file" "$HOME/.ssh_proxy/config.toml.bak" 2>/dev/null || true
fi
cat > "$config_file" <<EOF
[identity]
node_id = "$node_id"
node_name = "$node_name"
secret = {token}

[daemon]
control_endpoint = "tcp://$control"
transport_listen = "$transport"
token = {token}
route_autostart = true

[daemon.token_metadata]
created_at_unix = $created_at_unix
scope = "daemon-control-transport"
{peer_table}
EOF
chmod 600 "$config_file" 2>/dev/null || true
printf 'transport=%s\ncontrol=%s\nnode_id=%s\nnode_name=%s\nconfig=%s\n' "$transport" "$control" "$node_id" "$node_name" "$config_file"
"#,
        transport_port = preferred_transport.port(),
        control_port = preferred_control.port(),
        token = toml_quote(token),
        peer_table = peer_table
    )
}

pub(super) fn remote_systemd_install_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
) -> String {
    let escaped = sh_quote(remote_path);
    let token_arg = token_arg(args.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(args);
    format!(
        "set -eu; systemctl --user show-environment >/dev/null; if command -v loginctl >/dev/null 2>&1; then loginctl enable-linger \"$(id -un)\" >/dev/null 2>&1 || true; fi; mkdir -p ~/.config/systemd/user; cat > ~/.config/systemd/user/ssh-proxy-helper.service <<'EOF'\n[Unit]\nDescription=ssh_proxy node daemon\nAfter=network-online.target\nWants=network-online.target\nStartLimitIntervalSec=0\n[Service]\nExecStart={} node daemon --transport {} --control tcp://{}{}{}\nRestart=always\nRestartSec=3\nKillSignal=SIGINT\n[Install]\nWantedBy=default.target\nEOF\nsystemctl --user daemon-reload && systemctl --user enable --now ssh-proxy-helper.service",
        escaped, args.remote_tcp, args.remote_control, token_arg, extra_args
    )
}

pub(super) fn remote_nohup_start_command(
    remote_path: &str,
    args: &cli::InstallRemoteArgs,
    stop_existing: bool,
) -> String {
    let stop = if stop_existing {
        format!("{}; ", remote_nohup_stop_snippet(args.remote_tcp))
    } else {
        String::new()
    };
    let token_arg = token_arg(args.remote_token.as_deref());
    let extra_args = node_daemon_extra_args(args);
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(args.remote_tcp);
    format!(
        "set -eu; mkdir -p \"$HOME/.ssh_proxy/run\" \"$HOME/.ssh_proxy/log\"; {stop} cat > {scriptfile} <<'EOF'\n#!/bin/sh\nset -u\ntrap 'if [ -f {childfile} ]; then child=$(cat {childfile} 2>/dev/null || true); [ -n \"$child\" ] && kill \"$child\" 2>/dev/null || true; fi; exit 0' INT TERM\nbackoff=1\nwhile :; do\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) starting ssh_proxy node daemon\" >>{logfile}\n  {remote_path} node daemon --transport {remote_tcp} --control tcp://{remote_control}{token_arg}{extra_args} >>{logfile} 2>&1 &\n  child=$!\n  echo \"$child\" >{childfile}\n  wait \"$child\"\n  code=$?\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) ssh_proxy node daemon exited status=$code; restarting in ${{backoff}}s\" >>{logfile}\n  sleep \"$backoff\"\n  if [ \"$backoff\" -lt 30 ]; then backoff=$((backoff * 2)); fi\ndone\nEOF\nchmod 700 {scriptfile}; nohup /bin/sh {scriptfile} >/dev/null 2>&1 < /dev/null & echo $! > {pidfile}; sleep 1; pid=$(cat {pidfile}); kill -0 \"$pid\"",
        remote_path = sh_quote(remote_path),
        remote_tcp = args.remote_tcp,
        remote_control = args.remote_control,
        token_arg = token_arg,
        extra_args = extra_args,
        logfile = logfile,
        pidfile = pidfile,
        childfile = childfile,
        scriptfile = scriptfile,
    )
}

pub(super) fn remote_stop_command(remote_tcp: SocketAddr) -> String {
    format!(
        "set +e; systemctl --user stop ssh-proxy-helper.service >/dev/null 2>&1; {}; echo stopped",
        remote_nohup_stop_snippet(remote_tcp)
    )
}

pub(super) fn remote_restart_command(remote_path: &str, args: &cli::InstallRemoteArgs) -> String {
    format!(
        "set -eu; if systemctl --user status ssh-proxy-helper.service >/dev/null 2>&1; then systemctl --user restart ssh-proxy-helper.service; else {}; fi",
        remote_nohup_start_command(remote_path, args, true)
    )
}

pub(super) fn remote_logs_command(remote_tcp: SocketAddr, lines: usize) -> String {
    let (_, _, logfile, _) = remote_nohup_files(remote_tcp);
    let lines = lines.clamp(1, 5000);
    format!(
        "set +e; logfile={logfile}; journalctl --user -u ssh-proxy-helper.service -n {lines} --no-pager 2>/dev/null || true; if [ -f \"$logfile\" ]; then echo '--- nohup log ---'; tail -n {lines} \"$logfile\"; else echo \"no nohup log at $logfile\"; fi",
    )
}

pub(super) fn remote_clean_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let (_, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "set +e; {stop}; for pid in $(pgrep -f {remote_path_pattern} 2>/dev/null || true); do if [ \"$pid\" != \"$$\" ] && [ \"$pid\" != \"$PPID\" ]; then kill \"$pid\" >/dev/null 2>&1 || true; fi; done; systemctl --user disable --now ssh-proxy-helper.service >/dev/null 2>&1; rm -f ~/.config/systemd/user/ssh-proxy-helper.service; systemctl --user daemon-reload >/dev/null 2>&1; rm -f {remote_path} {logfile} {childfile} {scriptfile}; echo cleaned",
        stop = remote_stop_command(remote_tcp),
        remote_path_pattern = sh_quote(&format!("{remote_path} node daemon")),
        remote_path = sh_quote(remote_path),
        logfile = logfile,
        childfile = childfile,
        scriptfile = scriptfile,
    )
}

pub(super) fn remote_doctor_command(remote_path: &str, remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "set +e; echo 'ssh_proxy remote doctor'; echo user=$(id -un 2>/dev/null); echo uid=$(id -u 2>/dev/null); echo home=$HOME; echo shell=$SHELL; echo path=$PATH; echo uname=$(uname -a 2>/dev/null); echo pid1=$(ps -p 1 -o comm= 2>/dev/null); echo systemctl=$(command -v systemctl 2>/dev/null); echo nohup=$(command -v nohup 2>/dev/null); echo ss=$(command -v ss 2>/dev/null); echo remote_tcp={remote_tcp}; echo remote_path={remote_path}; if [ -x {remote_path_q} ]; then {remote_path_q} --version 2>/dev/null || true; else echo binary=missing-or-not-executable; fi; echo remote_path_on_path=$(command -v ssh_proxy 2>/dev/null || true); echo systemd_user_probe=$(systemctl --user show-environment >/dev/null 2>&1; echo $?); echo pidfile={pidfile}; echo childfile={childfile}; echo logfile={logfile}; echo scriptfile={scriptfile}; {status}; if command -v ss >/dev/null 2>&1; then ss -ltnp 2>/dev/null | grep ':{port} ' || true; fi",
        remote_path = remote_path,
        remote_path_q = sh_quote(remote_path),
        status = remote_nohup_status_snippet(remote_tcp),
        port = remote_tcp.port(),
        childfile = childfile,
        scriptfile = scriptfile,
    )
}

pub(super) fn remote_nohup_status_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; logfile={}; scriptfile={}; if [ -f \"$pidfile\" ]; then pid=$(cat \"$pidfile\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then echo \"supervisor running pid=$pid\"; else echo \"stale supervisor pidfile pid=$pid\"; fi; else echo \"not installed\"; fi; if [ -f \"$childfile\" ]; then child=$(cat \"$childfile\" 2>/dev/null || true); if [ -n \"$child\" ] && kill -0 \"$child\" 2>/dev/null; then echo \"helper running pid=$child\"; else echo \"stale helper child pid=$child\"; fi; fi; if [ -f \"$logfile\" ]; then echo \"log=$logfile\"; tail -n 20 \"$logfile\" 2>/dev/null || true; fi",
        pidfile, childfile, logfile, scriptfile
    )
}

pub(super) fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, _, _) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; for f in \"$childfile\" \"$pidfile\"; do if [ -f \"$f\" ]; then pid=$(cat \"$f\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\" 2>/dev/null || true; for i in 1 2 3 4 5; do kill -0 \"$pid\" 2>/dev/null || break; sleep 1; done; kill -9 \"$pid\" 2>/dev/null || true; fi; rm -f \"$f\"; fi; done",
        pidfile, childfile
    )
}

pub(super) fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    let port = remote_tcp.port();
    (
        format!("$HOME/.ssh_proxy/run/helper-{port}.pid"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.child.pid"),
        format!("$HOME/.ssh_proxy/log/helper-{port}.log"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.supervisor.sh"),
    )
}

pub(super) fn token_arg(token: Option<&str>) -> String {
    token
        .map(|token| format!(" --token {}", sh_quote(token)))
        .unwrap_or_default()
}

pub(super) fn node_daemon_extra_args(args: &cli::InstallRemoteArgs) -> String {
    let mut out = String::new();
    if let Some(addr) = args.remote_tls_transport {
        out.push_str(&format!(" --tls-transport {addr}"));
    }
    if let Some(addr) = args.remote_quic_transport {
        out.push_str(&format!(" --quic-transport {addr}"));
    }
    if let Some(path) = &args.remote_tls_cert {
        out.push_str(&format!(" --tls-cert {}", sh_quote(path)));
    }
    if let Some(path) = &args.remote_tls_key {
        out.push_str(&format!(" --tls-key {}", sh_quote(path)));
    }
    if let Some(path) = &args.remote_tls_client_ca {
        out.push_str(&format!(" --tls-client-ca {}", sh_quote(path)));
    }
    out
}
pub(super) fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn toml_quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_node_control_command_includes_explicit_token() {
        let command = remote_node_control_command(
            "/home/me/bin/ssh_proxy",
            "127.0.0.1:19081".parse().unwrap(),
            Some("secret token"),
            "status",
        );

        assert!(command.contains("--token 'secret token'"), "{command}");
        assert!(command.contains("node control --endpoint tcp://127.0.0.1:19081"));
    }

    #[test]
    fn remote_node_control_json_command_uses_send_with_token() {
        let request = serde_json::json!({"cmd": "routes"});
        let command = remote_node_control_json_command(
            "/home/me/bin/ssh_proxy",
            "127.0.0.1:19081".parse().unwrap(),
            Some("secret token"),
            &request,
        );

        assert!(command.contains("--token 'secret token'"), "{command}");
        assert!(command.contains(" send '"), "{command}");
    }
}
