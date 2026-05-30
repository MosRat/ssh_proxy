use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{
    command::{
        ChildGuard, control_status_via_tcp, direct_host_from_ssh_config, failure_class, free_addr,
        openssh_command, openssh_command_for_target, output_error, run_output, run_output_retry,
        run_with_stdin, russh_host_exec_command, scp_command, sh_quote, temp_dir, temp_path,
        wait_tcp,
    },
    config::{MatrixConfig, MatrixLevel, stamp},
    report::{MatrixCaseReport, MatrixReport},
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
    token: String,
    local_cert: PathBuf,
    local_key: PathBuf,
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
        sandbox.setup(config, report);
        for spec in case_specs(config, level) {
            sandbox.run_case(config, report, &spec);
        }
    });
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
            token: format!("matrix-{stamp}-{safe_target}"),
            local_cert,
            local_key,
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
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    fn setup(&self, config: &MatrixConfig, report: &mut MatrixReport) {
        self.generate_cert(config, report);
        self.upload_sidecar(config, report);
        self.start_daemon(config, report);
        self.assert_remote_status(config, report);
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

    fn run_case(&self, config: &MatrixConfig, report: &mut MatrixReport, spec: &MatrixCaseSpec) {
        if spec.direct_only && !config.is_direct_target(&self.target) {
            let mut row = self.case_row(config, spec);
            row.skip(
                "preflight_skip",
                "direct peer endpoints are skipped for non-direct topology",
            );
            row.fallback_classification = Some("preflight_skip".to_string());
            report.push(row);
            return;
        }

        let mut row = self.case_row(config, spec);
        let mut lost = 0_u64;
        let mut total_bytes = 0_u64;
        let mut total_duration = 0_u128;
        let mut first_byte = None;
        let measurements = self.run_proxy_measurements(config, spec);
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
                    if !control_response_ok(&measurement.response) {
                        lost += 1;
                        row.fail("runtime", unexpected_control_response(&measurement));
                    } else {
                        total_bytes += measurement.bytes;
                        total_duration += measurement.duration_ms;
                        first_byte = Some(
                            first_byte
                                .unwrap_or(measurement.first_byte_ms)
                                .min(measurement.first_byte_ms),
                        );
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
        if total_bytes > 0 {
            row.with_measurement(total_bytes, total_duration.max(1), first_byte.unwrap_or(0));
        }
        report.push(row);
    }

    fn run_proxy_measurements(
        &self,
        config: &MatrixConfig,
        spec: &MatrixCaseSpec,
    ) -> MatrixMeasurements {
        let listen = free_addr();
        let home = temp_dir("matrix-proxy-home");
        let stderr_path = temp_path("matrix-proxy-stderr", "log");
        let stderr = match fs::File::create(&stderr_path)
            .map_err(|err| format!("create proxy stderr log {}: {err}", stderr_path.display()))
        {
            Ok(file) => file,
            Err(err) => return MatrixMeasurements::single_error(config, err),
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
                &format!("127.0.0.1:{}", self.control_port),
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
            Err(err) => return MatrixMeasurements::single_error(config, err),
        };
        if let Err(err) = wait_tcp(listen, &mut child) {
            return MatrixMeasurements::single_error(config, with_proxy_log(err, &stderr_path));
        }

        let mut measurements = match config.requested {
            MatrixLevel::Stability => self.run_stability_samples(config, listen),
            MatrixLevel::PerfSmoke => self.run_perf_samples(config, listen, spec.samples),
            _ => {
                let started = Instant::now();
                MatrixMeasurements {
                    results: vec![control_status_via_tcp(listen, &self.token)],
                    measurement_scope: "control-status-through-proxy",
                    sample_count: 1,
                    request_count: 1,
                    concurrency: 1,
                    run_window_ms: started.elapsed().as_millis().max(1),
                }
            }
        };
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

    fn run_perf_samples(
        &self,
        config: &MatrixConfig,
        listen: SocketAddr,
        samples: usize,
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
            measurement_scope: "control-status-through-proxy",
            sample_count: samples as u64,
            request_count: (samples * concurrency) as u64,
            concurrency: concurrency as u64,
            run_window_ms: started.elapsed().as_millis().max(1),
        }
    }

    fn run_stability_samples(
        &self,
        config: &MatrixConfig,
        listen: SocketAddr,
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
            measurement_scope: "control-status-through-proxy",
            concurrency: 1,
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

    fn case_row(&self, config: &MatrixConfig, spec: &MatrixCaseSpec) -> MatrixCaseReport {
        MatrixCaseReport::new(
            config.level_name(),
            Some(&self.target),
            Some(&self.topology),
            spec.case,
        )
        .with_transport(
            spec.selected_transport,
            spec.selection_source,
            spec.selection_reason,
        )
    }

    fn remote_cleanup(&self) -> String {
        let command = format!(
            "set +e; if [ -f {pid} ]; then kill \"$(cat {pid})\" 2>/dev/null || true; fi; case {dir} in /tmp/ssh_proxy-matrix-*) rm -rf {dir} ;; *) echo refused-cleanup; exit 1 ;; esac",
            pid = sh_quote(&format!("{}/daemon.pid", self.remote_dir)),
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
    fn single_error(config: &MatrixConfig, error: String) -> Self {
        Self {
            results: vec![Err(error)],
            measurement_scope: "control-status-through-proxy",
            sample_count: 1,
            request_count: 1,
            concurrency: config.concurrency.max(1) as u64,
            run_window_ms: 0,
        }
    }
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
) {
    let mut row = MatrixCaseReport::new(config.level_name(), Some(target), Some(topology), case);
    match output {
        Ok(output) if output.status.success() => row.status = "passed".to_string(),
        Ok(output) => row.fail(failure_class(&output), output_error(&output)),
        Err(err) => row.fail(classify_command_error(&err), err),
    }
    report.push(row);
}

fn classify_command_error(error: &str) -> &'static str {
    if error.contains("failed to spawn") {
        "spawn_failed"
    } else {
        classify_runtime_error(error)
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

fn control_response_ok(response: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(response)
        .ok()
        .and_then(|value| value.get("ok").and_then(|ok| ok.as_bool()))
        .unwrap_or(false)
}

fn unexpected_control_response(measurement: &super::command::TcpMeasurement) -> String {
    append_proxy_log(
        &format!(
            "unexpected control response: {}",
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
    if candidate % 4 == 0 {
        candidate
    } else {
        candidate + (4 - candidate % 4)
    }
}
