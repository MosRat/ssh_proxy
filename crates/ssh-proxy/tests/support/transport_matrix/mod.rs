mod command;
mod config;
mod remote;
mod report;

use command::tool_available;
use config::{MatrixConfig, MatrixLevel};
use remote::{probe_target, run_target_matrix};
use report::{MatrixCaseReport, MatrixReport};

pub fn run_probe() {
    run(MatrixLevel::Probe, |config, report| {
        check_local_prerequisites(config, report);
        for target in &config.targets {
            probe_target(config, report, target);
        }
    });
}

pub fn run_smoke() {
    run(MatrixLevel::Smoke, |config, report| {
        check_local_prerequisites(config, report);
        for target in &config.targets {
            probe_target(config, report, target);
            run_target_matrix(config, report, target, MatrixLevel::Smoke);
        }
    });
}

pub fn run_perf_smoke() {
    run(MatrixLevel::PerfSmoke, |config, report| {
        check_local_prerequisites(config, report);
        for target in &config.targets {
            probe_target(config, report, target);
            run_target_matrix(config, report, target, MatrixLevel::PerfSmoke);
        }
    });
}

pub fn run_stability() {
    run(MatrixLevel::Stability, |config, report| {
        check_local_prerequisites(config, report);
        for target in &config.targets {
            probe_target(config, report, target);
            run_target_matrix(config, report, target, MatrixLevel::Stability);
        }
    });
}

fn run(level: MatrixLevel, test: impl FnOnce(&MatrixConfig, &mut MatrixReport)) {
    let Some(config) = MatrixConfig::load(level) else {
        return;
    };
    if !config.should_run(level) {
        return;
    }

    let mut report = MatrixReport::new(level, &config);
    test(&config, &mut report);
    let artifact = report.write();
    eprintln!("transport matrix report: {}", artifact.display());
    report.assert_no_hard_failures();
}

fn check_local_prerequisites(config: &MatrixConfig, report: &mut MatrixReport) {
    let mut release =
        MatrixCaseReport::new(config.level_name(), None, None, "local_release_binary");
    if config.local_bin.is_file() {
        release.status = "passed".to_string();
    } else {
        release.fail(
            "missing_release_binary",
            format!(
                "missing release binary {}; run `rtk cargo build -p ssh_proxy --release` or set SSH_PROXY_MATRIX_LOCAL_BIN",
                config.local_bin.display()
            ),
        );
    }
    report.push(release);

    let mut sidecar = MatrixCaseReport::new(config.level_name(), None, None, "linux_musl_sidecar");
    if config.sidecar.is_file() {
        sidecar.status = "passed".to_string();
    } else {
        sidecar.fail(
            "missing_sidecar",
            format!(
                "missing Linux sidecar {}; run `rtk cargo zigbuild -p ssh_proxy --target x86_64-unknown-linux-musl --release` or set SSH_PROXY_MATRIX_SIDECAR",
                config.sidecar.display()
            ),
        );
    }
    report.push(sidecar);

    for tool in ["ssh", "scp", "curl"] {
        let mut row = MatrixCaseReport::new(
            config.level_name(),
            None,
            None,
            &format!("local_tool_{tool}"),
        );
        if tool_available(tool) {
            row.status = "passed".to_string();
        } else {
            row.fail(
                "missing_tool",
                format!("{tool} was not found on PATH for transport matrix tests"),
            );
        }
        report.push(row);
    }
}
