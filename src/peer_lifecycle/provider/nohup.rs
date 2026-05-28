use std::net::SocketAddr;

use crate::cli;

use super::util::{node_daemon_extra_args, remote_mark_service_state_command, sh_quote, token_arg};

pub(crate) fn remote_nohup_start_command(
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
        "set -eu; mkdir -p \"$HOME/.ssh_proxy/run\" \"$HOME/.ssh_proxy/log\"; {stop} cat > {scriptfile} <<'EOF'\n#!/bin/sh\nset -u\ntrap 'if [ -f {childfile} ]; then child=$(cat {childfile} 2>/dev/null || true); [ -n \"$child\" ] && kill \"$child\" 2>/dev/null || true; fi; exit 0' INT TERM\nbackoff=1\nwhile :; do\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) starting ssh_proxy node daemon\" >>{logfile}\n  {remote_path} node daemon --transport {remote_tcp} --control tcp://{remote_control}{token_arg}{extra_args} >>{logfile} 2>&1 &\n  child=$!\n  echo \"$child\" >{childfile}\n  wait \"$child\"\n  code=$?\n  echo \"$(date -u +%Y-%m-%dT%H:%M:%SZ) ssh_proxy node daemon exited status=$code; restarting in ${{backoff}}s\" >>{logfile}\n  sleep \"$backoff\"\n  if [ \"$backoff\" -lt 30 ]; then backoff=$((backoff * 2)); fi\ndone\nEOF\nchmod 700 {scriptfile}; nohup /bin/sh {scriptfile} >/dev/null 2>&1 < /dev/null & echo $! > {pidfile}; sleep 1; pid=$(cat {pidfile}); kill -0 \"$pid\"; {mark}",
        remote_path = sh_quote(remote_path),
        remote_tcp = args.remote_tcp,
        remote_control = args.remote_control,
        token_arg = token_arg,
        extra_args = extra_args,
        logfile = logfile,
        pidfile = pidfile,
        childfile = childfile,
        scriptfile = scriptfile,
        mark = remote_mark_service_state_command("nohup_supervisor", "healthy", "start_service"),
    )
}

pub(crate) fn remote_nohup_status_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, logfile, scriptfile) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; logfile={}; scriptfile={}; if [ -f \"$pidfile\" ]; then pid=$(cat \"$pidfile\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then echo \"supervisor running pid=$pid\"; else echo \"stale supervisor pidfile pid=$pid\"; fi; else echo \"not installed\"; fi; if [ -f \"$childfile\" ]; then child=$(cat \"$childfile\" 2>/dev/null || true); if [ -n \"$child\" ] && kill -0 \"$child\" 2>/dev/null; then echo \"helper running pid=$child\"; else echo \"stale helper child pid=$child\"; fi; fi; if [ -f \"$logfile\" ]; then echo \"log=$logfile\"; tail -n 20 \"$logfile\" 2>/dev/null || true; fi",
        pidfile, childfile, logfile, scriptfile
    )
}

pub(crate) fn remote_nohup_stop_snippet(remote_tcp: SocketAddr) -> String {
    let (pidfile, childfile, _, _) = remote_nohup_files(remote_tcp);
    format!(
        "pidfile={}; childfile={}; for f in \"$childfile\" \"$pidfile\"; do if [ -f \"$f\" ]; then pid=$(cat \"$f\" 2>/dev/null || true); if [ -n \"$pid\" ] && kill -0 \"$pid\" 2>/dev/null; then kill \"$pid\" 2>/dev/null || true; for i in 1 2 3 4 5; do kill -0 \"$pid\" 2>/dev/null || break; sleep 1; done; kill -9 \"$pid\" 2>/dev/null || true; fi; rm -f \"$f\"; fi; done",
        pidfile, childfile
    )
}

pub(crate) fn remote_nohup_files(remote_tcp: SocketAddr) -> (String, String, String, String) {
    let port = remote_tcp.port();
    (
        format!("$HOME/.ssh_proxy/run/helper-{port}.pid"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.child.pid"),
        format!("$HOME/.ssh_proxy/log/helper-{port}.log"),
        format!("$HOME/.ssh_proxy/run/helper-{port}.supervisor.sh"),
    )
}
