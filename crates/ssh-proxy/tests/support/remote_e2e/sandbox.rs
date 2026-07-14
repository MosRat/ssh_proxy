use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    command::{
        assert_stdout_contains, assert_success, failure_class, json_string,
        openssh_command_for_target, run_output, scp_command, sh_quote,
    },
    config::{RemoteConfig, env_string},
};

const SIDECAR_ENV: &str = "SSH_PROXY_REMOTE_SIDECAR";

#[derive(Debug, Clone)]
pub(super) struct RemoteSandbox {
    target: String,
    topology: String,
    accept_new: bool,
    remote_dir: String,
    remote_bin: String,
    remote_transport: String,
    remote_control: String,
    token: String,
    sidecar: PathBuf,
}

impl RemoteSandbox {
    pub(super) fn new(target: &str, config: &RemoteConfig) -> Self {
        let stamp = stamp();
        let safe_target = sanitize_alias(target);
        let remote_dir = format!("/tmp/ssh_proxy-e2e-{stamp}-{safe_target}");
        let remote_bin = format!("{remote_dir}/ssh_proxy");
        let base_port = allocate_remote_base_port(&stamp, target);
        let token = format!("e2e-{stamp}-{safe_target}");
        Self {
            target: target.to_string(),
            topology: config.topology_for(target).to_string(),
            accept_new: config.accept_new,
            remote_dir,
            remote_bin,
            remote_transport: format!("127.0.0.1:{base_port}"),
            remote_control: format!("127.0.0.1:{}", base_port + 1),
            token,
            sidecar: sidecar_path(),
        }
    }

    pub(super) fn with_cleanup(&self, keep: bool, test: impl FnOnce(&Self)) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(self)));
        if keep {
            eprintln!(
                "remote e2e kept: target={} topology={} dir={}",
                self.target, self.topology, self.remote_dir
            );
        } else {
            self.remote_cleanup();
        }
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    pub(super) fn upload_sidecar(&self) {
        assert!(
            self.sidecar.is_file(),
            "remote e2e sidecar missing at {}; run `rtk cargo zigbuild -p ssh_proxy --target x86_64-unknown-linux-musl --release` or set {SIDECAR_ENV}",
            self.sidecar.display()
        );

        let mkdir = format!("mkdir -p {}", sh_quote(&self.remote_dir));
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&mkdir],
        ));
        assert_success(
            "remote_mkdir",
            &self.target,
            &self.topology,
            &output,
            "failed to create remote sandbox",
        );

        let copy = run_output(scp_command(
            &self.sidecar,
            &self.target,
            self.accept_new,
            &self.remote_bin,
        ));
        assert_success(
            "remote_upload",
            &self.target,
            &self.topology,
            &copy,
            "failed to upload sidecar",
        );

        let chmod = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&format!("chmod 700 {}", sh_quote(&self.remote_bin))],
        ));
        assert_success(
            "remote_chmod",
            &self.target,
            &self.topology,
            &chmod,
            "failed to chmod sidecar",
        );
    }

    pub(super) fn start_daemon(&self) {
        let command = format!(
            "mkdir -p {home} {run}; SSH_PROXY_HOME={home} nohup {bin} --log warn node daemon --transport {transport} --control tcp://{control} --token {token} --routes-path {routes} >{log} 2>&1 < /dev/null & echo $! > {pid}",
            home = sh_quote(&format!("{}/home", self.remote_dir)),
            run = sh_quote(&format!("{}/run", self.remote_dir)),
            bin = sh_quote(&self.remote_bin),
            transport = self.remote_transport,
            control = self.remote_control,
            token = sh_quote(&self.token),
            routes = sh_quote(&format!("{}/routes.json", self.remote_dir)),
            log = sh_quote(&format!("{}/daemon.log", self.remote_dir)),
            pid = sh_quote(&format!("{}/daemon.pid", self.remote_dir)),
        );
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        ));
        assert_success(
            "remote_daemon_start",
            &self.target,
            &self.topology,
            &output,
            "failed to start remote daemon",
        );
    }

    pub(super) fn assert_remote_control_status(&self) {
        let command = format!(
            "for i in $(seq 1 40); do {bin} --log warn node control --endpoint tcp://{control} --token {token} --json status >/tmp/ssh_proxy-e2e-status.$$ 2>/tmp/ssh_proxy-e2e-status.err.$$ && cat /tmp/ssh_proxy-e2e-status.$$ && rm -f /tmp/ssh_proxy-e2e-status.$$ /tmp/ssh_proxy-e2e-status.err.$$ && exit 0; sleep 0.25; done; cat /tmp/ssh_proxy-e2e-status.err.$$ 2>/dev/null || true; echo 'remote daemon status timed out'; cat {log} 2>/dev/null || true; exit 1",
            bin = sh_quote(&self.remote_bin),
            control = self.remote_control,
            token = sh_quote(&self.token),
            log = sh_quote(&format!("{}/daemon.log", self.remote_dir)),
        );
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        ));
        assert_success(
            "remote_daemon_status",
            &self.target,
            &self.topology,
            &output,
            "remote daemon did not become ready",
        );
        assert_stdout_contains(&output, "\"ok\":true", "remote daemon status JSON");
    }

    pub(super) fn assert_remote_control_routes(&self) {
        let command = format!(
            "{bin} --log warn node control --endpoint tcp://{control} --token {token} --json routes",
            bin = sh_quote(&self.remote_bin),
            control = self.remote_control,
            token = sh_quote(&self.token),
        );
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        ));
        assert_success(
            "remote_daemon_routes",
            &self.target,
            &self.topology,
            &output,
            "remote daemon routes command failed",
        );
        assert_stdout_contains(&output, "\"ok\":true", "remote daemon routes JSON");
    }

    pub(super) fn assert_remote_admin_checksum(&self) {
        let intent = format!(
            r#"{{"command":"checksum","path":{}}}"#,
            json_string(&self.remote_bin)
        );
        let command = format!(
            "printf %s {intent} | {bin} remote admin",
            intent = sh_quote(&intent),
            bin = sh_quote(&self.remote_bin),
        );
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        ));
        assert_success(
            "remote_admin_checksum",
            &self.target,
            &self.topology,
            &output,
            "remote admin checksum failed",
        );
        assert_stdout_contains(
            &output,
            "\"execution_backend\":\"own_binary\"",
            "checksum backend",
        );
        assert_stdout_contains(&output, "\"fallback_used\":false", "checksum fallback");
    }

    pub(super) fn assert_remote_admin_status(&self) {
        let intent = format!(
            r#"{{"command":"status","remote_tcp":{},"remote_path":{}}}"#,
            json_string(&self.remote_transport),
            json_string(&self.remote_bin)
        );
        let command = format!(
            "printf %s {intent} | {bin} remote admin",
            intent = sh_quote(&intent),
            bin = sh_quote(&self.remote_bin),
        );
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        ));
        assert_success(
            "remote_admin_status",
            &self.target,
            &self.topology,
            &output,
            "remote admin status failed",
        );
        assert_stdout_contains(
            &output,
            "\"execution_backend\":\"own_binary\"",
            "status backend",
        );
        assert_stdout_contains(&output, "\"fallback_used\":false", "status fallback");
    }

    fn remote_cleanup(&self) {
        let command = format!(
            "set +e; if [ -f {pid} ]; then kill \"$(cat {pid})\" 2>/dev/null || true; fi; case {dir} in /tmp/ssh_proxy-e2e-*) rm -rf {dir} ;; *) echo refused-cleanup ;; esac",
            pid = sh_quote(&format!("{}/daemon.pid", self.remote_dir)),
            dir = sh_quote(&self.remote_dir),
        );
        let output = run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        ));
        if output.status.success() {
            eprintln!(
                "remote e2e cleanup ok: target={} topology={}",
                self.target, self.topology
            );
        } else {
            eprintln!(
                "remote e2e cleanup failed: target={} topology={} classification={}",
                self.target,
                self.topology,
                failure_class(&output)
            );
        }
    }
}

fn sidecar_path() -> PathBuf {
    if let Some(path) = env_string(SIDECAR_ENV) {
        return PathBuf::from(path);
    }
    workspace_root()
        .join("target")
        .join("x86_64-unknown-linux-musl")
        .join("release")
        .join("ssh_proxy")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("ssh_proxy package should live under crates/ssh-proxy")
        .to_path_buf()
}

fn stamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    format!("{millis}-{}", std::process::id())
}

fn sanitize_alias(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn allocate_remote_base_port(stamp: &str, target: &str) -> u16 {
    let mut hash = 0_u32;
    for byte in stamp.bytes().chain(target.bytes()) {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(byte));
    }
    let candidate = 23000 + (hash % 18000) as u16;
    if candidate % 2 == 0 {
        candidate
    } else {
        candidate + 1
    }
}
