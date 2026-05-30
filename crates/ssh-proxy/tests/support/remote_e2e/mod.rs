mod command;
mod config;
mod sandbox;

use command::{
    assert_success, openssh_command, run_output, run_with_stdin, russh_host_exec_command,
};
use config::{RemoteConfig, RemoteLevel};
use sandbox::RemoteSandbox;

pub fn run_probe() {
    let Some(config) = RemoteConfig::load(RemoteLevel::Probe) else {
        return;
    };
    config.run(RemoteLevel::Probe, |config| {
        for target in &config.targets {
            probe_target(config, target);
        }
    });
}

pub fn run_smoke() {
    let Some(config) = RemoteConfig::load(RemoteLevel::Smoke) else {
        return;
    };
    config.run(RemoteLevel::Smoke, |config| {
        for target in &config.targets {
            probe_target(config, target);
            let sandbox = RemoteSandbox::new(target, config);
            sandbox.with_cleanup(config.keep, |sandbox| {
                sandbox.upload_sidecar();
                sandbox.start_daemon();
                sandbox.assert_remote_control_status();
                sandbox.assert_remote_control_routes();
            });
        }
    });
}

pub fn run_full() {
    let Some(config) = RemoteConfig::load(RemoteLevel::Full) else {
        return;
    };
    config.run(RemoteLevel::Full, |config| {
        for target in &config.targets {
            probe_target(config, target);
            let sandbox = RemoteSandbox::new(target, config);
            sandbox.with_cleanup(config.keep, |sandbox| {
                sandbox.upload_sidecar();
                sandbox.start_daemon();
                sandbox.assert_remote_control_status();
                sandbox.assert_remote_admin_checksum();
                sandbox.assert_remote_admin_status();
            });
        }
    });
}

fn probe_target(config: &RemoteConfig, target: &str) {
    let topology = config.topology_for(target);
    let openssh = run_output(openssh_command(
        target,
        config.accept_new,
        "printf 'openssh:ok\n'",
    ));
    assert_success(
        "openssh_probe",
        target,
        topology,
        &openssh,
        "OpenSSH reachability probe failed",
    );

    let script = "printf 'russh:ok\n'";
    let russh = run_with_stdin(
        russh_host_exec_command(
            target,
            config.accept_new,
            config.upstream_proxy.as_deref(),
            "remote-probe",
        ),
        script,
    );
    assert_success(
        "russh_probe",
        target,
        topology,
        &russh,
        "ssh_proxy russh host exec probe failed",
    );
}
