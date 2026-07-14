use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{
    bench::{
        BENCH_SERVER_SCRIPT, bench_download_via_tcp, bench_stream_via_tcp, bench_upload_via_tcp,
    },
    command::{
        ChildGuard, control_status_via_tcp, direct_host_from_ssh_config, failure_class, free_addr,
        openssh_command, openssh_command_for_target, output_error, run_output, run_output_retry,
        run_output_retry_timeout, run_output_timeout, run_with_stdin, russh_host_exec_command,
        scp_command, sh_quote, temp_dir, temp_path, wait_tcp,
    },
    config::{MatrixConfig, MatrixLevel, stamp},
    report::{MatrixCaseReport, MatrixReport, MeasurementSample},
    workload::MatrixWorkload,
};

#[derive(Debug)]
struct RemoteMatrixSandbox {
    target: String,
    topology: String,
    direct_host: String,
    remote_dir: String,
    remote_bin: String,
    remote_cert: String,
    remote_key: String,
    control_port: u16,
    plain_port: u16,
    tls_port: u16,
    quic_port: u16,
    bench_port: u16,
    token: String,
    local_cert: PathBuf,
    local_key: PathBuf,
    local_bench_script: PathBuf,
    accept_new: bool,
}

#[derive(Debug, Clone)]
struct MatrixCaseSpec {
    case: &'static str,
    selected_transport: &'static str,
    selection_source: &'static str,
    selection_reason: &'static str,
    direct_only: bool,
    samples: usize,
}

struct MatrixMeasurements {
    results: Vec<Result<super::command::TcpMeasurement, String>>,
    measurement_scope: &'static str,
    sample_count: u64,
    request_count: u64,
    concurrency: u64,
    run_window_ms: u128,
}

#[derive(Debug, Clone, Copy)]
struct SetupState {
    bench_ready: bool,
}

pub(super) fn probe_target(config: &MatrixConfig, report: &mut MatrixReport, target: &str) {
    let topology = config.topology_for(target);
    let mut openssh = MatrixCaseReport::new(
        config.level_name(),
        Some(target),
        Some(topology),
        "openssh_probe",
    );
    match run_output_retry(
        || openssh_command(target, config.accept_new, "printf '%s\\n' openssh:ok"),
        3,
    ) {
        Ok(output) if output.status.success() => {
            openssh.status = "passed".to_string();
        }
        Ok(output) => openssh.fail(failure_class(&output), output_error(&output)),
        Err(err) => openssh.fail(classify_command_error(&err), err),
    }
    report.push(openssh);

    let mut russh = MatrixCaseReport::new(
        config.level_name(),
        Some(target),
        Some(topology),
        "russh_probe",
    );
    match run_with_stdin(
        russh_host_exec_command(&config.local_bin, target, config.accept_new, "matrix-probe"),
        "printf '%s\\n' russh:ok",
    ) {
        Ok(output) if output.status.success() => {
            russh.status = "passed".to_string();
        }
        Ok(output) => russh.fail(failure_class(&output), output_error(&output)),
        Err(err) => russh.fail(classify_command_error(&err), err),
    }
    report.push(russh);

    let stamp = stamp();
    let remote_dir = format!(
        "/tmp/ssh_proxy-matrix-probe-{stamp}-{}",
        sanitize_alias(target)
    );
    let probe = format!(
        "mkdir -p {dir}; test -d {dir}; rmdir {dir}",
        dir = sh_quote(&remote_dir)
    );
    let mut tmp = MatrixCaseReport::new(
        config.level_name(),
        Some(target),
        Some(topology),
        "remote_tmp_probe",
    );
    match run_output_retry(|| openssh_command(target, config.accept_new, &probe), 3) {
        Ok(output) if output.status.success() => tmp.status = "passed".to_string(),
        Ok(output) => tmp.fail(failure_class(&output), output_error(&output)),
        Err(err) => tmp.fail(classify_command_error(&err), err),
    }
    report.push(tmp);
}

pub(super) fn run_target_matrix(
    config: &MatrixConfig,
    report: &mut MatrixReport,
    target: &str,
    level: MatrixLevel,
) {
    let sandbox = RemoteMatrixSandbox::new(config, target);
    sandbox.with_cleanup(config.keep, report, |sandbox, report| {
        let setup = sandbox.setup(config, report);
        for spec in case_specs(config, level) {
            for workload in &config.workloads {
                sandbox.run_case(config, report, &spec, *workload, setup);
            }
        }
    });
}

pub(super) fn cleanup_target(config: &MatrixConfig, report: &mut MatrixReport, target: &str) {
    let topology = config.topology_for(target);
    let command = "set +e; for pid in /tmp/ssh_proxy-matrix-*/daemon.pid /tmp/ssh_proxy-matrix-*/bench.pid; do if [ -f \"$pid\" ]; then kill \"$(cat \"$pid\")\" 2>/dev/null || true; fi; done; find /tmp -maxdepth 1 -type d -name 'ssh_proxy-matrix-*' -exec rm -rf -- {} +";
    let mut row = MatrixCaseReport::new(
        config.level_name(),
        Some(target),
        Some(topology),
        "remote_cleanup_sweep",
    );
    row.cleanup_status = Some("requested".to_string());
    match run_output(openssh_command_for_target(
        target,
        config.accept_new,
        &[command],
    )) {
        Ok(output) if output.status.success() => {
            row.cleanup_status = Some("ok".to_string());
            row.status = "passed".to_string();
        }
        Ok(output) => {
            row.cleanup_status = Some("failed".to_string());
            row.fail(failure_class(&output), output_error(&output));
        }
        Err(err) => {
            row.cleanup_status = Some("failed".to_string());
            row.fail(classify_command_error(&err), err);
        }
    }
    report.push(row);
}

impl RemoteMatrixSandbox {
    fn new(config: &MatrixConfig, target: &str) -> Self {
        let stamp = stamp();
        let safe_target = sanitize_alias(target);
        let remote_dir = format!("/tmp/ssh_proxy-matrix-{stamp}-{safe_target}");
        let base_port = allocate_remote_base_port(&stamp, target);
        let local_cert = temp_path("matrix-cert", "pem");
        let local_key = temp_path("matrix-key", "pem");
        Self {
            target: target.to_string(),
            topology: config.topology_for(target).to_string(),
            direct_host: direct_host_from_ssh_config(target),
            remote_bin: format!("{remote_dir}/ssh_proxy"),
            remote_cert: format!("{remote_dir}/cert.pem"),
            remote_key: format!("{remote_dir}/key.pem"),
            remote_dir,
            control_port: base_port,
            plain_port: base_port + 1,
            tls_port: base_port + 2,
            quic_port: base_port + 3,
            bench_port: base_port + 4,
            token: format!("matrix-{stamp}-{safe_target}"),
            local_cert,
            local_key,
            local_bench_script: temp_path("matrix-bench-server", "py"),
            accept_new: config.accept_new,
        }
    }

    fn with_cleanup(
        &self,
        keep: bool,
        report: &mut MatrixReport,
        test: impl FnOnce(&Self, &mut MatrixReport),
    ) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(self, report)));
        let cleanup_status = if keep {
            eprintln!(
                "transport matrix kept remote state: target={} topology={} dir={}",
                self.target, self.topology, self.remote_dir
            );
            "kept".to_string()
        } else {
            self.remote_cleanup()
        };
        let mut row = MatrixCaseReport::new(
            "cleanup",
            Some(&self.target),
            Some(&self.topology),
            "remote_cleanup",
        );
        row.cleanup_status = Some(cleanup_status.clone());
        if cleanup_status == "failed" {
            row.fail("cleanup_failed", "remote cleanup command failed");
        }
        report.push(row);
        let _ = fs::remove_file(&self.local_cert);
        let _ = fs::remove_file(&self.local_key);
        let _ = fs::remove_file(&self.local_bench_script);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    fn setup(&self, config: &MatrixConfig, report: &mut MatrixReport) -> SetupState {
        self.generate_cert(config, report);
        self.upload_sidecar(config, report);
        self.start_daemon(config, report);
        self.assert_remote_status(config, report);
        let bench_ready = if config.needs_bench_server() {
            self.start_bench_server(config, report)
        } else {
            false
        };
        SetupState { bench_ready }
    }

    fn generate_cert(&self, config: &MatrixConfig, report: &mut MatrixReport) {
        let mut row = MatrixCaseReport::new(
            config.level_name(),
            Some(&self.target),
            Some(&self.topology),
            "generate_rcgen_cert",
        );
        match rcgen::generate_simple_self_signed(vec!["localhost".to_string()]) {
            Ok(certified) => {
                let cert = certified.cert.pem();
                let key = certified.signing_key.serialize_pem();
                if let Err(err) = fs::write(&self.local_cert, cert) {
                    row.fail("local_io", format!("write cert: {err}"));
                } else if let Err(err) = fs::write(&self.local_key, key) {
                    row.fail("local_io", format!("write key: {err}"));
                }
            }
            Err(err) => row.fail("cert_generation", err.to_string()),
        }
        report.push(row);
    }

    fn upload_sidecar(&self, config: &MatrixConfig, report: &mut MatrixReport) {
        let mkdir = format!("mkdir -p {}", sh_quote(&self.remote_dir));
        push_command_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_mkdir",
            run_output(openssh_command_for_target(
                &self.target,
                config.accept_new,
                &[&mkdir],
            )),
        );

        for (case, local, remote) in [
            (
                "remote_matrix_upload_sidecar",
                config.sidecar.as_path(),
                self.remote_bin.as_str(),
            ),
            (
                "remote_matrix_upload_cert",
                self.local_cert.as_path(),
                self.remote_cert.as_str(),
            ),
            (
                "remote_matrix_upload_key",
                self.local_key.as_path(),
                self.remote_key.as_str(),
            ),
        ] {
            push_command_case(
                config,
                report,
                &self.target,
                &self.topology,
                case,
                run_output(scp_command(local, &self.target, config.accept_new, remote)),
            );
        }

        let chmod = format!(
            "chmod 700 {bin}; chmod 600 {key}; chmod 644 {cert}",
            bin = sh_quote(&self.remote_bin),
            key = sh_quote(&self.remote_key),
            cert = sh_quote(&self.remote_cert),
        );
        push_command_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_chmod",
            run_output(openssh_command_for_target(
                &self.target,
                config.accept_new,
                &[&chmod],
            )),
        );
    }

    fn start_daemon(&self, config: &MatrixConfig, report: &mut MatrixReport) {
        let command = format!(
            "mkdir -p {home}; SSH_PROXY_HOME={home} nohup {bin} --log warn node daemon --control tcp://127.0.0.1:{control} --transport 0.0.0.0:{plain} --tls-transport 0.0.0.0:{tls} --quic-transport 0.0.0.0:{quic} --tls-cert {cert} --tls-key {key} --token {token} --routes-path {routes} --no-route-autostart >{log} 2>&1 < /dev/null & echo $! > {pid}",
            home = sh_quote(&format!("{}/home", self.remote_dir)),
            bin = sh_quote(&self.remote_bin),
            control = self.control_port,
            plain = self.plain_port,
            tls = self.tls_port,
            quic = self.quic_port,
            cert = sh_quote(&self.remote_cert),
            key = sh_quote(&self.remote_key),
            token = sh_quote(&self.token),
            routes = sh_quote(&format!("{}/routes.json", self.remote_dir)),
            log = sh_quote(&format!("{}/daemon.log", self.remote_dir)),
            pid = sh_quote(&format!("{}/daemon.pid", self.remote_dir)),
        );
        push_command_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_daemon_start",
            run_output(openssh_command_for_target(
                &self.target,
                config.accept_new,
                &[&command],
            )),
        );
    }

    fn assert_remote_status(&self, config: &MatrixConfig, report: &mut MatrixReport) {
        let command = format!(
            "for i in $(seq 1 40); do {bin} --log warn node control --endpoint tcp://127.0.0.1:{control} --token {token} --json status >/tmp/ssh_proxy-matrix-status.$$ 2>/tmp/ssh_proxy-matrix-status.err.$$ && cat /tmp/ssh_proxy-matrix-status.$$ && rm -f /tmp/ssh_proxy-matrix-status.$$ /tmp/ssh_proxy-matrix-status.err.$$ && exit 0; sleep 0.25; done; cat /tmp/ssh_proxy-matrix-status.err.$$ 2>/dev/null || true; cat {log} 2>/dev/null || true; exit 1",
            bin = sh_quote(&self.remote_bin),
            control = self.control_port,
            token = sh_quote(&self.token),
            log = sh_quote(&format!("{}/daemon.log", self.remote_dir)),
        );
        push_command_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_daemon_status",
            run_output(openssh_command_for_target(
                &self.target,
                config.accept_new,
                &[&command],
            )),
        );
    }

    fn start_bench_server(&self, config: &MatrixConfig, report: &mut MatrixReport) -> bool {
        let mut python = MatrixCaseReport::new(
            config.level_name(),
            Some(&self.target),
            Some(&self.topology),
            "remote_matrix_python3",
        );
        match run_output(openssh_command_for_target(
            &self.target,
            config.accept_new,
            &["command -v python3 >/dev/null 2>&1"],
        )) {
            Ok(output) if output.status.success() => python.status = "passed".to_string(),
            Ok(output) => {
                python.skip(
                    failure_class(&output),
                    "python3 is not available for matrix payload workloads",
                );
                report.push(python);
                return false;
            }
            Err(err) => {
                python.skip(
                    classify_command_error(&err),
                    "python3 availability probe failed for matrix payload workloads",
                );
                report.push(python);
                return false;
            }
        }
        report.push(python);

        let mut local_script = MatrixCaseReport::new(
            config.level_name(),
            Some(&self.target),
            Some(&self.topology),
            "local_matrix_bench_script",
        );
        if let Err(err) = fs::write(&self.local_bench_script, BENCH_SERVER_SCRIPT) {
            local_script.fail("local_io", format!("write bench server script: {err}"));
            report.push(local_script);
            return false;
        }
        report.push(local_script);

        let remote_script = format!("{}/bench_server.py", self.remote_dir);
        if !push_command_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_upload_bench_server",
            run_output(scp_command(
                &self.local_bench_script,
                &self.target,
                config.accept_new,
                &remote_script,
            )),
        ) {
            return false;
        }

        let wait_code = r#"import socket,sys,time
port=int(sys.argv[1])
deadline=time.time()+10
while time.time()<deadline:
    try:
        s=socket.create_connection(("127.0.0.1", port), timeout=0.25)
        s.close()
        sys.exit(0)
    except OSError:
        time.sleep(0.25)
sys.exit(1)
"#;
        let start_command = format!(
            "rm -f {pid}; nohup python3 {script} {port} >{log} 2>&1 < /dev/null & echo $! > {pid}; sleep 0.2; test -s {pid}",
            script = sh_quote(&remote_script),
            port = self.bench_port,
            log = sh_quote(&format!("{}/bench.log", self.remote_dir)),
            pid = sh_quote(&format!("{}/bench.pid", self.remote_dir)),
        );
        if !push_bench_setup_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_bench_server_start",
            run_output_timeout(
                openssh_command_for_target(&self.target, config.accept_new, &[&start_command]),
                Duration::from_secs(20),
            ),
        ) {
            return false;
        }

        let probe_command = format!(
            "python3 -c {wait} {port} || (cat {log} 2>/dev/null; exit 1)",
            wait = sh_quote(wait_code),
            port = self.bench_port,
            log = sh_quote(&format!("{}/bench.log", self.remote_dir)),
        );
        let mut make_probe =
            || openssh_command_for_target(&self.target, config.accept_new, &[&probe_command]);
        push_bench_setup_case(
            config,
            report,
            &self.target,
            &self.topology,
            "remote_matrix_bench_server_probe",
            run_output_retry_timeout(&mut make_probe, 5, Duration::from_secs(15)),
        )
    }

    fn run_case(
        &self,
        config: &MatrixConfig,
        report: &mut MatrixReport,
        spec: &MatrixCaseSpec,
        workload: MatrixWorkload,
        setup: SetupState,
    ) {
        if spec.direct_only && !config.is_direct_target(&self.target) {
            let mut row = self.case_row(config, spec, workload);
            row.skip(
                "preflight_skip",
                "direct peer endpoints are skipped for non-direct topology",
            );
            row.fallback_classification = Some("preflight_skip".to_string());
            report.push(row);
            return;
        }
        if workload.requires_bench_server() && !setup.bench_ready {
            let mut row = self.case_row(config, spec, workload);
            row.skip(
                "missing_remote_bench_server",
                "payload workload skipped because remote bench server is unavailable",
            );
            row.fallback_classification = Some("preflight_skip".to_string());
            report.push(row);
            return;
        }

        let mut row = self.case_row(config, spec, workload);
        row.payload_bytes = payload_bytes_for(config, workload);
        let mut lost = 0_u64;
        let mut successful_samples = Vec::new();
        let measurements = if spec.case == "openssh-direct-tcpip" {
            self.run_openssh_forward_measurements(config, workload)
        } else {
            self.run_proxy_measurements(config, spec, workload)
        };
        row.with_measurement_context(
            measurements.measurement_scope,
            measurements.sample_count,
            measurements.request_count,
            measurements.concurrency,
            measurements.run_window_ms,
        );
        for result in measurements.results {
            match result {
                Ok(measurement) => {
                    if !measurement_ok(workload, &measurement) {
                        lost += 1;
                        row.fail(
                            "runtime",
                            unexpected_workload_response(workload, &measurement),
                        );
                    } else {
                        successful_samples.push(MeasurementSample {
                            bytes: measurement.bytes,
                            duration_ms: measurement.duration_ms,
                            first_byte_ms: measurement.first_byte_ms,
                        });
                    }
                }
                Err(err) => {
                    lost += 1;
                    row.fail(classify_runtime_error(&err), err);
                }
            }
        }
        row.lost_requests = Some(lost);
        row.reconnect_count = Some(0);
        if !successful_samples.is_empty() {
            row.with_measurement_samples(&successful_samples);
        }
        report.push(row);
    }

    fn run_openssh_forward_measurements(
        &self,
        config: &MatrixConfig,
        workload: MatrixWorkload,
    ) -> MatrixMeasurements {
        let mut last = None;
        for attempt in 0..3 {
            let measurements = self.run_openssh_forward_measurements_once(config, workload);
            if attempt == 2 || !measurements_should_retry(&measurements) {
                return measurements;
            }
            last = Some(measurements);
            thread::sleep(Duration::from_millis(250));
        }
        last.expect("OpenSSH forward retry loop should run at least once")
    }

    fn run_openssh_forward_measurements_once(
        &self,
        config: &MatrixConfig,
        workload: MatrixWorkload,
    ) -> MatrixMeasurements {
        let listen = free_addr();
        let target_port = self.target_port_for(workload);
        let stderr_path = temp_path("matrix-openssh-stderr", "log");
        let stderr = match fs::File::create(&stderr_path)
            .map_err(|err| format!("create OpenSSH stderr log {}: {err}", stderr_path.display()))
        {
            Ok(file) => file,
            Err(err) => {
                return MatrixMeasurements::single_error(
                    config,
                    workload.measurement_scope("openssh"),
                    err,
                );
            }
        };
        let mut command = Command::new("ssh");
        command
            .arg("-N")
            .arg("-T")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=10")
            .arg("-o")
            .arg(if config.accept_new {
                "StrictHostKeyChecking=accept-new"
            } else {
                "StrictHostKeyChecking=yes"
            })
            .arg("-L")
            .arg(format!("{}:127.0.0.1:{}", listen, target_port))
            .arg(&self.target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        let mut child = match command
            .spawn()
            .map(ChildGuard::new)
            .map_err(|err| format!("start OpenSSH forward: {err}"))
        {
            Ok(child) => child,
            Err(err) => {
                return MatrixMeasurements::single_error(
                    config,
                    workload.measurement_scope("openssh"),
                    err,
                );
            }
        };
        if let Err(err) = wait_tcp(listen, &mut child) {
            return MatrixMeasurements::single_error(
                config,
                workload.measurement_scope("openssh"),
                with_proxy_log(err, &stderr_path),
            );
        }

        let mut measurements =
            self.run_workload_measurements(config, listen, workload, "openssh", config.samples);
        child.kill_and_wait();
        let proxy_log = read_proxy_log(&stderr_path);
        for result in &mut measurements.results {
            if let Err(err) = result {
                let current = std::mem::take(err);
                *err = append_proxy_log(&current, proxy_log.as_deref());
            }
        }
        measurements
    }

    fn run_proxy_measurements(
        &self,
        config: &MatrixConfig,
        spec: &MatrixCaseSpec,
        workload: MatrixWorkload,
    ) -> MatrixMeasurements {
        let mut last = None;
        for attempt in 0..3 {
            let measurements = self.run_proxy_measurements_once(config, spec, workload);
            if attempt == 2 || !measurements_should_retry(&measurements) {
                return measurements;
            }
            last = Some(measurements);
            thread::sleep(Duration::from_millis(250));
        }
        last.expect("proxy measurement retry loop should run at least once")
    }

    fn run_proxy_measurements_once(
        &self,
        config: &MatrixConfig,
        spec: &MatrixCaseSpec,
        workload: MatrixWorkload,
    ) -> MatrixMeasurements {
        let listen = free_addr();
        let target_port = self.target_port_for(workload);
        let home = temp_dir("matrix-proxy-home");
        let stderr_path = temp_path("matrix-proxy-stderr", "log");
        let stderr = match fs::File::create(&stderr_path)
            .map_err(|err| format!("create proxy stderr log {}: {err}", stderr_path.display()))
        {
            Ok(file) => file,
            Err(err) => {
                return MatrixMeasurements::single_error(
                    config,
                    workload.measurement_scope("proxy"),
                    err,
                );
            }
        };
        let mut command = Command::new(&config.local_bin);
        command
            .args([
                "--log",
                "warn",
                "proxy",
                &self.target,
                "--listen",
                &listen.to_string(),
                "--remote-transport",
                spec.selected_transport,
                "--tcp-target",
                &format!("127.0.0.1:{target_port}"),
                "--connect-timeout-secs",
                "20",
                "--no-reconnect",
                "--deploy",
                "never",
            ])
            .env("SSH_PROXY_HOME", home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        if config.accept_new {
            command.arg("--accept-new");
        }
        self.add_transport_args(&mut command, spec);
        let mut child = match command
            .spawn()
            .map(ChildGuard::new)
            .map_err(|err| format!("start local proxy: {err}"))
        {
            Ok(child) => child,
            Err(err) => {
                return MatrixMeasurements::single_error(
                    config,
                    workload.measurement_scope("proxy"),
                    err,
                );
            }
        };
        if let Err(err) = wait_tcp(listen, &mut child) {
            return MatrixMeasurements::single_error(
                config,
                workload.measurement_scope("proxy"),
                with_proxy_log(err, &stderr_path),
            );
        }

        let mut measurements =
            self.run_workload_measurements(config, listen, workload, "proxy", spec.samples);
        child.kill_and_wait();
        let proxy_log = read_proxy_log(&stderr_path);
        for result in &mut measurements.results {
            match result {
                Ok(measurement) => measurement.proxy_stderr = proxy_log.clone(),
                Err(err) => {
                    let current = std::mem::take(err);
                    *err = append_proxy_log(&current, proxy_log.as_deref());
                }
            }
        }
        measurements
    }

    fn run_workload_measurements(
        &self,
        config: &MatrixConfig,
        listen: SocketAddr,
        workload: MatrixWorkload,
        backend: &'static str,
        samples: usize,
    ) -> MatrixMeasurements {
        let scope = workload.measurement_scope(backend);
        match workload {
            MatrixWorkload::Control => match config.requested {
                MatrixLevel::Stability => self.run_control_stability_samples(config, listen, scope),
                MatrixLevel::PerfSmoke => {
                    self.run_control_perf_samples(config, listen, samples, scope)
                }
                _ => {
                    let started = Instant::now();
                    MatrixMeasurements {
                        results: vec![control_status_via_tcp(listen, &self.token)],
                        measurement_scope: scope,
                        sample_count: 1,
                        request_count: 1,
                        concurrency: 1,
                        run_window_ms: started.elapsed().as_millis().max(1),
                    }
                }
            },
            MatrixWorkload::LargeDownload => {
                let started = Instant::now();
                let samples = samples.max(1);
                let mut results = Vec::with_capacity(samples);
                for _ in 0..samples {
                    results.push(bench_download_via_tcp(listen, config.payload_bytes));
                }
                MatrixMeasurements {
                    results,
                    measurement_scope: scope,
                    sample_count: samples as u64,
                    request_count: samples as u64,
                    concurrency: 1,
                    run_window_ms: started.elapsed().as_millis().max(1),
                }
            }
            MatrixWorkload::LargeUpload => {
                let started = Instant::now();
                let samples = samples.max(1);
                let mut results = Vec::with_capacity(samples);
                for _ in 0..samples {
                    results.push(bench_upload_via_tcp(listen, config.payload_bytes));
                }
                MatrixMeasurements {
                    results,
                    measurement_scope: scope,
                    sample_count: samples as u64,
                    request_count: samples as u64,
                    concurrency: 1,
                    run_window_ms: started.elapsed().as_millis().max(1),
                }
            }
            MatrixWorkload::LongConnection => {
                let started = Instant::now();
                MatrixMeasurements {
                    results: vec![bench_stream_via_tcp(listen, config.long_connection_secs)],
                    measurement_scope: scope,
                    sample_count: 1,
                    request_count: 1,
                    concurrency: 1,
                    run_window_ms: started.elapsed().as_millis().max(1),
                }
            }
            MatrixWorkload::HighConcurrency => {
                self.run_high_concurrency_samples(config, listen, scope)
            }
        }
    }

    fn run_control_perf_samples(
        &self,
        config: &MatrixConfig,
        listen: SocketAddr,
        samples: usize,
        scope: &'static str,
    ) -> MatrixMeasurements {
        let mut results = Vec::new();
        let started = Instant::now();
        let samples = samples.max(1);
        let concurrency = config.concurrency.max(1);
        for _ in 0..samples {
            let mut handles = Vec::new();
            let batch_started = Instant::now();
            for _ in 0..concurrency {
                let token = self.token.clone();
                handles.push(thread::spawn(move || {
                    control_status_via_tcp(listen, &token)
                }));
            }
            let mut successes = Vec::new();
            for handle in handles {
                match handle
                    .join()
                    .unwrap_or_else(|_| Err("matrix perf worker panicked".to_string()))
                {
                    Ok(measurement) => successes.push(measurement),
                    Err(err) => results.push(Err(err)),
                }
            }
            if !successes.is_empty() {
                results.push(Ok(aggregate_batch_measurement(
                    successes,
                    batch_started.elapsed().as_millis().max(1),
                )));
            }
        }
        MatrixMeasurements {
            results,
            measurement_scope: scope,
            sample_count: samples as u64,
            request_count: (samples * concurrency) as u64,
            concurrency: concurrency as u64,
            run_window_ms: started.elapsed().as_millis().max(1),
        }
    }

    fn run_control_stability_samples(
        &self,
        config: &MatrixConfig,
        listen: SocketAddr,
        scope: &'static str,
    ) -> MatrixMeasurements {
        let started = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(config.duration_secs.max(1));
        let mut results = Vec::new();
        while Instant::now() < deadline {
            results.push(control_status_via_tcp(listen, &self.token));
            thread::sleep(Duration::from_secs(5));
        }
        if results.is_empty() {
            results.push(control_status_via_tcp(listen, &self.token));
        }
        MatrixMeasurements {
            request_count: results.len() as u64,
            sample_count: results.len() as u64,
            results,
            measurement_scope: scope,
            concurrency: 1,
            run_window_ms: started.elapsed().as_millis().max(1),
        }
    }

    fn run_high_concurrency_samples(
        &self,
        config: &MatrixConfig,
        listen: SocketAddr,
        scope: &'static str,
    ) -> MatrixMeasurements {
        let started = Instant::now();
        let samples = config.samples.max(1);
        let concurrency = config.concurrency.max(1);
        let mut results = Vec::new();
        for _ in 0..samples {
            let batch_started = Instant::now();
            let mut handles = Vec::new();
            for _ in 0..concurrency {
                let bytes = config.concurrent_payload_bytes;
                handles.push(thread::spawn(move || bench_download_via_tcp(listen, bytes)));
            }
            let mut successes = Vec::new();
            for handle in handles {
                match handle
                    .join()
                    .unwrap_or_else(|_| Err("matrix concurrency worker panicked".to_string()))
                {
                    Ok(measurement) => successes.push(measurement),
                    Err(err) => results.push(Err(err)),
                }
            }
            if !successes.is_empty() {
                results.push(Ok(aggregate_batch_measurement(
                    successes,
                    batch_started.elapsed().as_millis().max(1),
                )));
            }
        }
        MatrixMeasurements {
            results,
            measurement_scope: scope,
            sample_count: samples as u64,
            request_count: (samples * concurrency) as u64,
            concurrency: concurrency as u64,
            run_window_ms: started.elapsed().as_millis().max(1),
        }
    }

    fn add_transport_args(&self, command: &mut Command, spec: &MatrixCaseSpec) {
        match spec.case {
            "ssh-native" => {}
            "spx-over-ssh" => {
                command
                    .arg("--remote-tcp")
                    .arg(format!("127.0.0.1:{}", self.plain_port))
                    .arg("--remote-token")
                    .arg(&self.token);
            }
            "spx-plain-direct" => {
                command
                    .arg("--remote-tcp")
                    .arg(format!("{}:{}", self.direct_host, self.plain_port))
                    .arg("--allow-plain-tcp")
                    .arg("--remote-token")
                    .arg(&self.token);
            }
            "spx-tls-direct" => {
                command
                    .arg("--remote-tls")
                    .arg(format!("{}:{}", self.direct_host, self.tls_port))
                    .arg("--remote-ca")
                    .arg(&self.local_cert)
                    .arg("--remote-name")
                    .arg("localhost")
                    .arg("--remote-token")
                    .arg(&self.token);
            }
            "spx-quic-direct" => {
                command
                    .arg("--remote-quic")
                    .arg(format!("{}:{}", self.direct_host, self.quic_port))
                    .arg("--remote-ca")
                    .arg(&self.local_cert)
                    .arg("--remote-name")
                    .arg("localhost")
                    .arg("--remote-token")
                    .arg(&self.token);
            }
            "quic-native-direct" => {
                command
                    .arg("--remote-quic")
                    .arg(format!("{}:{}", self.direct_host, self.quic_port))
                    .arg("--remote-ca")
                    .arg(&self.local_cert)
                    .arg("--remote-name")
                    .arg("localhost");
            }
            _ => {}
        }
    }

    fn case_row(
        &self,
        config: &MatrixConfig,
        spec: &MatrixCaseSpec,
        workload: MatrixWorkload,
    ) -> MatrixCaseReport {
        MatrixCaseReport::new(
            config.level_name(),
            Some(&self.target),
            Some(&self.topology),
            spec.case,
        )
        .with_workload(workload.as_str())
        .with_transport(
            spec.selected_transport,
            spec.selection_source,
            spec.selection_reason,
        )
    }

    fn target_port_for(&self, workload: MatrixWorkload) -> u16 {
        if workload.requires_bench_server() {
            self.bench_port
        } else {
            self.control_port
        }
    }

    fn remote_cleanup(&self) -> String {
        let command = format!(
            "set +e; for pid in {daemon_pid} {bench_pid}; do if [ -f \"$pid\" ]; then kill \"$(cat \"$pid\")\" 2>/dev/null || true; fi; done; case {dir} in /tmp/ssh_proxy-matrix-*) rm -rf {dir} ;; *) echo refused-cleanup; exit 1 ;; esac",
            daemon_pid = sh_quote(&format!("{}/daemon.pid", self.remote_dir)),
            bench_pid = sh_quote(&format!("{}/bench.pid", self.remote_dir)),
            dir = sh_quote(&self.remote_dir),
        );
        match run_output(openssh_command_for_target(
            &self.target,
            self.accept_new,
            &[&command],
        )) {
            Ok(output) if output.status.success() => "ok".to_string(),
            _ => "failed".to_string(),
        }
    }
}

impl MatrixMeasurements {
    fn single_error(config: &MatrixConfig, scope: &'static str, error: String) -> Self {
        Self {
            results: vec![Err(error)],
            measurement_scope: scope,
            sample_count: 1,
            request_count: 1,
            concurrency: config.concurrency.max(1) as u64,
            run_window_ms: 0,
        }
    }
}

fn measurements_should_retry(measurements: &MatrixMeasurements) -> bool {
    !measurements.results.is_empty()
        && measurements
            .results
            .iter()
            .all(|result| matches!(result, Err(err) if transient_runtime_error(err)))
}

fn transient_runtime_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("banner exchange")
        || lower.contains("connection timed out")
        || lower.contains("operation timed out")
        || lower.contains("timed out")
        || lower.contains("connection closed")
        || lower.contains("broken pipe")
}

fn aggregate_batch_measurement(
    measurements: Vec<super::command::TcpMeasurement>,
    duration_ms: u128,
) -> super::command::TcpMeasurement {
    let mut iter = measurements.into_iter();
    let first = iter
        .next()
        .expect("aggregate requires at least one measurement");
    let mut response = first.response.clone();
    let mut bytes = first.bytes;
    let mut first_byte_ms = first.first_byte_ms;
    for measurement in iter {
        if response.is_empty() {
            response = measurement.response.clone();
        }
        bytes += measurement.bytes;
        first_byte_ms = first_byte_ms.min(measurement.first_byte_ms);
    }
    super::command::TcpMeasurement {
        response,
        bytes,
        duration_ms,
        first_byte_ms,
        proxy_stderr: None,
    }
}

fn case_specs(config: &MatrixConfig, level: MatrixLevel) -> Vec<MatrixCaseSpec> {
    let samples = match level {
        MatrixLevel::PerfSmoke => config.samples.max(1),
        MatrixLevel::Stability => 0,
        _ => 1,
    };
    if level == MatrixLevel::Stability {
        return stability_specs();
    }
    vec![
        MatrixCaseSpec {
            case: "openssh-direct-tcpip",
            selected_transport: "openssh-direct-tcpip",
            selection_source: "matrix",
            selection_reason: "OpenSSH local forward direct-tcpip baseline",
            direct_only: false,
            samples,
        },
        MatrixCaseSpec {
            case: "ssh-native",
            selected_transport: "ssh-native",
            selection_source: "matrix",
            selection_reason: "rust ssh direct-tcpip baseline",
            direct_only: false,
            samples,
        },
        MatrixCaseSpec {
            case: "spx-over-ssh",
            selected_transport: "tcp",
            selection_source: "matrix",
            selection_reason: "SPX peer transport reached through SSH direct-tcpip",
            direct_only: false,
            samples,
        },
        MatrixCaseSpec {
            case: "spx-plain-direct",
            selected_transport: "plain-tcp",
            selection_source: "matrix",
            selection_reason: "explicit direct plain TCP peer transport",
            direct_only: true,
            samples,
        },
        MatrixCaseSpec {
            case: "spx-tls-direct",
            selected_transport: "tls-tcp",
            selection_source: "matrix",
            selection_reason: "direct TLS peer transport",
            direct_only: true,
            samples,
        },
        MatrixCaseSpec {
            case: "spx-quic-direct",
            selected_transport: "quic",
            selection_source: "matrix",
            selection_reason: "direct QUIC SPX peer transport",
            direct_only: true,
            samples,
        },
        MatrixCaseSpec {
            case: "quic-native-direct",
            selected_transport: "quic-native",
            selection_source: "matrix",
            selection_reason: "native QUIC per-flow data plane remains experimental",
            direct_only: true,
            samples,
        },
    ]
}

fn stability_specs() -> Vec<MatrixCaseSpec> {
    vec![
        MatrixCaseSpec {
            case: "openssh-direct-tcpip",
            selected_transport: "openssh-direct-tcpip",
            selection_source: "matrix",
            selection_reason: "OpenSSH local forward stability baseline",
            direct_only: false,
            samples: 1,
        },
        MatrixCaseSpec {
            case: "spx-over-ssh",
            selected_transport: "tcp",
            selection_source: "matrix",
            selection_reason: "stability baseline over SSH direct-tcpip",
            direct_only: false,
            samples: 1,
        },
        MatrixCaseSpec {
            case: "spx-tls-direct",
            selected_transport: "tls-tcp",
            selection_source: "matrix",
            selection_reason: "direct TLS stability sample",
            direct_only: true,
            samples: 1,
        },
        MatrixCaseSpec {
            case: "spx-quic-direct",
            selected_transport: "quic",
            selection_source: "matrix",
            selection_reason: "direct QUIC stability sample",
            direct_only: true,
            samples: 1,
        },
        MatrixCaseSpec {
            case: "quic-native-direct",
            selected_transport: "quic-native",
            selection_source: "matrix",
            selection_reason: "native QUIC experimental stability sample",
            direct_only: true,
            samples: 1,
        },
    ]
}

fn push_command_case(
    config: &MatrixConfig,
    report: &mut MatrixReport,
    target: &str,
    topology: &str,
    case: &str,
    output: Result<std::process::Output, String>,
) -> bool {
    let mut row = MatrixCaseReport::new(config.level_name(), Some(target), Some(topology), case);
    match output {
        Ok(output) if output.status.success() => row.status = "passed".to_string(),
        Ok(output) => row.fail(failure_class(&output), output_error(&output)),
        Err(err) => row.fail(classify_command_error(&err), err),
    }
    let passed = row.status == "passed";
    report.push(row);
    passed
}

fn push_bench_setup_case(
    config: &MatrixConfig,
    report: &mut MatrixReport,
    target: &str,
    topology: &str,
    case: &str,
    output: Result<std::process::Output, String>,
) -> bool {
    let mut row = MatrixCaseReport::new(config.level_name(), Some(target), Some(topology), case);
    match output {
        Ok(output) if output.status.success() => row.status = "passed".to_string(),
        Ok(output) => {
            let error = output_error(&output);
            row.skip(classify_bench_setup_error(&error), error);
            row.fallback_classification = Some("preflight_skip".to_string());
        }
        Err(err) => {
            row.skip(classify_bench_setup_error(&err), err);
            row.fallback_classification = Some("preflight_skip".to_string());
        }
    }
    let passed = row.status == "passed";
    report.push(row);
    passed
}

fn classify_command_error(error: &str) -> &'static str {
    if error.contains("failed to spawn") {
        "spawn_failed"
    } else {
        classify_runtime_error(error)
    }
}

fn classify_bench_setup_error(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("command timed out") {
        "bench_setup_timeout"
    } else if lower.contains("address already in use") {
        "bench_setup_port_conflict"
    } else if lower.contains("connection closed")
        || lower.contains("broken pipe")
        || lower.contains("banner exchange")
    {
        "transient_network"
    } else if lower.contains("python") && lower.contains("not found") {
        "missing_python3"
    } else {
        "bench_setup_unavailable"
    }
}

fn classify_runtime_error(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("cert") {
        "cert"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "network_timeout"
    } else if lower.contains("refused") {
        "connection_refused"
    } else if lower.contains("handshake") || lower.contains("protocol") {
        "protocol"
    } else if lower.contains("permission denied") || lower.contains("publickey") {
        "auth"
    } else {
        "runtime"
    }
}

fn payload_bytes_for(config: &MatrixConfig, workload: MatrixWorkload) -> Option<u64> {
    match workload {
        MatrixWorkload::Control | MatrixWorkload::LongConnection => None,
        MatrixWorkload::LargeDownload | MatrixWorkload::LargeUpload => Some(config.payload_bytes),
        MatrixWorkload::HighConcurrency => Some(config.concurrent_payload_bytes),
    }
}

fn measurement_ok(workload: MatrixWorkload, measurement: &super::command::TcpMeasurement) -> bool {
    match workload {
        MatrixWorkload::Control => control_response_ok(&measurement.response),
        MatrixWorkload::LargeDownload => measurement.response.starts_with("bench_download:"),
        MatrixWorkload::LargeUpload => measurement.response.starts_with("OK "),
        MatrixWorkload::LongConnection => measurement.response.starts_with("bench_stream:"),
        MatrixWorkload::HighConcurrency => measurement.response.starts_with("bench_download:"),
    }
}

fn control_response_ok(response: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(response)
        .ok()
        .and_then(|value| value.get("ok").and_then(|ok| ok.as_bool()))
        .unwrap_or(false)
}

fn unexpected_workload_response(
    workload: MatrixWorkload,
    measurement: &super::command::TcpMeasurement,
) -> String {
    append_proxy_log(
        &format!(
            "unexpected {} response: {}",
            workload.as_str(),
            nonempty_or_placeholder(&measurement.response)
        ),
        measurement.proxy_stderr.as_deref(),
    )
}

fn with_proxy_log(error: String, stderr_path: &PathBuf) -> String {
    append_proxy_log(&error, read_proxy_log(stderr_path).as_deref())
}

fn append_proxy_log(error: &str, proxy_log: Option<&str>) -> String {
    match proxy_log.map(str::trim).filter(|log| !log.is_empty()) {
        Some(log) => format!("{error}; proxy_stderr={log}"),
        None => error.to_string(),
    }
}

fn read_proxy_log(path: &PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|log| log.trim().to_string())
        .filter(|log| !log.is_empty())
}

fn nonempty_or_placeholder(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "<empty>"
    } else {
        trimmed
    }
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
    let candidate = 26000 + (hash % 16000) as u16;
    if candidate % 8 == 0 {
        candidate
    } else {
        candidate + (8 - candidate % 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_setup_errors_are_preflight_classified() {
        assert_eq!(
            classify_bench_setup_error("command timed out after 20s"),
            "bench_setup_timeout"
        );
        assert_eq!(
            classify_bench_setup_error("OSError: Address already in use"),
            "bench_setup_port_conflict"
        );
        assert_eq!(
            classify_bench_setup_error("Connection closed by remote host"),
            "transient_network"
        );
        assert_eq!(
            classify_bench_setup_error("python3: not found"),
            "missing_python3"
        );
    }

    #[test]
    fn workload_response_checks_distinguish_payload_shapes() {
        let measurement = super::super::command::TcpMeasurement {
            response: "bench_download:1048576".to_string(),
            bytes: 1024 * 1024,
            duration_ms: 10,
            first_byte_ms: 1,
            proxy_stderr: None,
        };
        assert!(measurement_ok(MatrixWorkload::LargeDownload, &measurement));
        assert!(measurement_ok(
            MatrixWorkload::HighConcurrency,
            &measurement
        ));
        assert!(!measurement_ok(MatrixWorkload::LargeUpload, &measurement));
    }
}
