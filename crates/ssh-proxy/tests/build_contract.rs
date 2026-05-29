use std::{fs, path::Path};

#[test]
fn cargo_manifest_keeps_release_binary_contract() {
    let manifest = read_repo_file("Cargo.toml");
    let package_manifest = read_package_file("Cargo.toml");

    assert_contains(
        &package_manifest,
        "\nmimalloc.workspace = true",
        "ssh_proxy package should keep mimalloc as a direct dependency",
    );
    assert_contains(
        &manifest,
        "\nlicense = \"MIT\"",
        "Cargo.toml should declare the repository license",
    );
    assert_contains(
        &manifest,
        "\nreadme = \"README.md\"",
        "Cargo.toml should point at the root README",
    );
    assert_contains(
        &manifest,
        "[profile.release]",
        "release profile should be explicit",
    );
    assert_contains(
        &manifest,
        "lto = \"fat\"",
        "release builds should keep fat LTO",
    );
    assert_contains(
        &manifest,
        "codegen-units = 1",
        "release builds should optimize as one codegen unit",
    );
    assert_contains(
        &manifest,
        "panic = \"abort\"",
        "release builds should avoid unwinding runtime cost",
    );
    assert_contains(
        &manifest,
        "strip = true",
        "release builds should strip binaries",
    );
}

#[test]
fn main_installs_mimalloc_global_allocator() {
    let main_rs = read_package_file("src/main.rs");

    assert_contains(
        &main_rs,
        "use mimalloc::MiMalloc;",
        "main should import mimalloc",
    );
    assert_contains(
        &main_rs,
        "#[global_allocator]",
        "main should define a global allocator",
    );
    assert_contains(
        &main_rs,
        "static GLOBAL: MiMalloc = MiMalloc;",
        "main should install mimalloc as the global allocator",
    );
}

#[test]
fn manifest_avoids_direct_c_ffi_crates() {
    let manifest = read_repo_file("Cargo.toml");
    let forbidden = [
        "bindgen",
        "cc",
        "cmake",
        "libc",
        "libssh2-sys",
        "openssl",
        "openssl-sys",
        "pkg-config",
    ];

    for name in forbidden {
        assert!(
            !manifest_has_direct_crate(&manifest, name),
            "Cargo.toml should not add direct C FFI/build dependency `{name}` without documenting why no Rust-native option works"
        );
    }
}

#[test]
fn source_avoids_direct_c_ffi_surface() {
    let mut violations = Vec::new();
    collect_workspace_source_ffi_violations(&mut violations);

    assert!(
        violations.is_empty(),
        "Rust source should avoid direct C FFI unless a plan documents the exception:\n{}",
        violations.join("\n")
    );
}

#[test]
fn release_docs_list_full_local_gate_commands() {
    let docs = read_repo_file("docs/release.md");
    let gates = [
        "cargo test --workspace --tests",
        "cargo build -p ssh_proxy --release",
        "cargo zigbuild -p ssh_proxy --target x86_64-unknown-linux-musl --release",
        "npm --prefix apps/vscode-remote-proxy test",
        "npm --prefix apps/vscode-remote-proxy run package:with-kernel",
    ];

    for gate in gates {
        assert_contains(
            &docs,
            gate,
            "release docs should list the local production gate",
        );
    }
}

#[test]
fn fast_check_scripts_keep_acceleration_contract() {
    let ps1 = read_repo_file("scripts/check-fast.ps1");
    let sh = read_repo_file("scripts/check-fast.sh");
    let docs = read_repo_file("docs/release.md");

    for text in [&ps1, &sh] {
        assert_contains(
            text,
            "check --workspace --tests",
            "fast check path should compile tests before running them",
        );
        assert_contains(
            text,
            "nextest run --workspace --tests",
            "fast check path should prefer cargo-nextest when available",
        );
        assert_contains(
            text,
            "test --workspace --tests",
            "fast check path should fall back to cargo test",
        );
        assert_contains(
            text,
            "sccache",
            "fast check path should document or use sccache acceleration",
        );
    }

    for command in [
        "cargo check --workspace --tests",
        "cargo nextest run --workspace --tests",
        "cargo test --workspace --tests",
        "sccache",
    ] {
        assert_contains(
            &docs,
            command,
            "release docs should describe the fast check path",
        );
    }
}

#[test]
fn workspace_crate_boundaries_remain_layered() {
    let root = read_repo_file("Cargo.toml");
    assert_contains(
        &root,
        "members = [\"crates/*\"]",
        "root manifest should keep workspace members under crates",
    );
    assert_contains(
        &root,
        "default-members = [\"crates/ssh-proxy\"]",
        "workspace should default to the ssh_proxy binary package",
    );

    assert_manifest_avoids(
        "crates/ssh-proxy/Cargo.toml",
        &[
            "russh",
            "quinn",
            "tokio-rustls",
            "windows-service",
            "windows-sys",
            "service-manager",
        ],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-core/Cargo.toml",
        &["tokio", "russh", "clap", "windows-service"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-protocol/Cargo.toml",
        &["clap", "russh", "ssh-proxy-lifecycle", "ssh-proxy-config"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-lifecycle/Cargo.toml",
        &["clap", "russh", "windows-service", "ssh-proxy"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-config/Cargo.toml",
        &["clap", "russh", "tokio", "windows-service", "ssh-proxy"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-cli/Cargo.toml",
        &["russh", "tokio", "windows-service", "ssh-proxy"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-control/Cargo.toml",
        &["clap", "russh", "windows-service", "ssh-proxy"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-ssh/Cargo.toml",
        &["clap", "windows-service", "ssh-proxy"],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-service/Cargo.toml",
        &[
            "clap",
            "russh",
            "tokio",
            "windows-service",
            "ssh-proxy",
            "service-manager",
        ],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-route/Cargo.toml",
        &[
            "clap",
            "russh",
            "tokio",
            "windows-service",
            "ssh-proxy",
            "service-manager",
        ],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-deploy/Cargo.toml",
        &[
            "clap",
            "russh",
            "tokio",
            "windows-service",
            "ssh-proxy",
            "service-manager",
        ],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-daemon/Cargo.toml",
        &[
            "clap",
            "russh",
            "tokio",
            "windows-service",
            "ssh-proxy",
            "service-manager",
        ],
    );
    assert_manifest_avoids(
        "crates/ssh-proxy-platform/Cargo.toml",
        &[
            "clap",
            "russh",
            "quinn",
            "tokio-rustls",
            "ssh-proxy",
            "service-manager",
        ],
    );
}

#[test]
fn workspace_members_do_not_depend_on_service_manager() {
    let crates_dir = workspace_root().join("crates");
    let entries = fs::read_dir(&crates_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", crates_dir.display()));

    for entry in entries {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                crates_dir.display()
            )
        });
        let manifest_path = entry.path().join("Cargo.toml");
        if manifest_path.is_file() {
            let manifest = fs::read_to_string(&manifest_path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest_path.display()));
            assert!(
                !manifest_has_direct_crate(&manifest, "service-manager"),
                "{} should keep service-manager out of production dependencies",
                manifest_path.display()
            );
        }
    }
}

#[test]
fn workspace_source_imports_remain_layered() {
    let app_imports = [
        "crate::cli::",
        "crate::{cli",
        "ssh_proxy_cli",
        "ssh_proxy::",
    ];
    let ssh_runtime_imports = ["use russh", "russh::"];
    let quic_runtime_imports = ["use quinn", "quinn::"];
    let service_runtime_imports = ["use windows_service", "windows_service::"];

    for member in [
        "ssh-proxy-core",
        "ssh-proxy-protocol",
        "ssh-proxy-control",
        "ssh-proxy-config",
        "ssh-proxy-lifecycle",
        "ssh-proxy-route",
        "ssh-proxy-deploy",
        "ssh-proxy-daemon",
    ] {
        assert_source_avoids(
            &format!("crates/{member}/src"),
            &[
                &app_imports[..],
                &ssh_runtime_imports[..],
                &quic_runtime_imports[..],
                &service_runtime_imports[..],
            ]
            .concat(),
        );
    }

    assert_source_avoids(
        "crates/ssh-proxy-transport/src",
        &[
            &app_imports[..],
            &ssh_runtime_imports[..],
            &service_runtime_imports[..],
        ]
        .concat(),
    );
    assert_source_avoids(
        "crates/ssh-proxy-ssh/src",
        &[
            &app_imports[..],
            &quic_runtime_imports[..],
            &service_runtime_imports[..],
        ]
        .concat(),
    );
    assert_source_avoids(
        "crates/ssh-proxy-service/src",
        &[
            &app_imports[..],
            &ssh_runtime_imports[..],
            &quic_runtime_imports[..],
        ]
        .concat(),
    );
    assert_source_avoids(
        "crates/ssh-proxy-cli/src",
        &[
            &ssh_runtime_imports[..],
            &quic_runtime_imports[..],
            &service_runtime_imports[..],
        ]
        .concat(),
    );
    assert_source_avoids(
        "crates/ssh-proxy-platform/src",
        &[
            &app_imports[..],
            &ssh_runtime_imports[..],
            &quic_runtime_imports[..],
        ]
        .concat(),
    );
}

#[test]
fn runtime_control_uses_command_neutral_intents() {
    let daemon_control = read_repo_file("crates/ssh-proxy-daemon/src/control.rs");
    assert_contains(
        &daemon_control,
        "pub struct NodeRequestIntent",
        "daemon crate should own command-neutral request intents",
    );
    assert_contains(
        &daemon_control,
        "pub enum NodeRequestPayload",
        "daemon crate should own typed request payload summaries",
    );

    let app_control =
        read_repo_file("crates/ssh-proxy/src/node_daemon/control_protocol/payload_adapter.rs");
    assert_contains(
        &app_control,
        "pub(crate) fn typed_intent(&self) -> NodeRequestIntent",
        "app control protocol should adapt legacy JSON to typed intents",
    );
    assert_contains(
        &app_control,
        "NodeRequestPayload::RouteStart",
        "legacy route payload fields should map into command-neutral payloads",
    );

    let server = read_repo_file("crates/ssh-proxy/src/node_daemon/control_server.rs");
    assert_contains(
        &server,
        "request.typed_intent()",
        "control server should parse the typed view before dispatch",
    );
}

#[test]
fn self_update_execution_goes_through_platform_plans() {
    let update = read_repo_file("crates/ssh-proxy/src/node_daemon/management/update.rs");
    assert_contains(
        &update,
        "PlatformScriptPlan",
        "self-update switch scripts should be described as platform plans",
    );
    assert_contains(
        &update,
        "ExternalActionClass::SelfUpdate",
        "self-update external execution should carry explicit classification",
    );
    assert_contains(
        &update,
        "ssh_proxy_platform::spawn_command",
        "self-update script launch should go through the platform crate",
    );
    assert_contains(
        &update,
        "ssh_proxy_platform::capture_command",
        "self-update version probing should go through the platform crate",
    );
    assert_not_contains(
        &update,
        "Command::new(",
        "self-update should not spawn commands directly from daemon runtime code",
    );
}

#[test]
fn remote_setup_execution_plans_stay_in_deploy_crate() {
    let deploy_setup = read_repo_file("crates/ssh-proxy-deploy/src/remote_setup.rs");
    assert_contains(
        &deploy_setup,
        "pub struct RemoteSetupExecutionPlan",
        "deploy crate should own remote setup execution plans",
    );
    assert_contains(
        &deploy_setup,
        "pub struct RemoteArtifactIntent",
        "deploy crate should own artifact write intents",
    );
    assert_contains(
        &deploy_setup,
        "cat > \\\"$tmp\\\"",
        "remote artifact writes should use stdin file writes",
    );
    let deploy_setup_production = deploy_setup.split("#[cfg(test)]").next().unwrap_or("");
    assert_not_contains(
        deploy_setup_production,
        "<<",
        "remote setup plans should not embed heredoc payloads",
    );

    let app_payload = read_repo_file("crates/ssh-proxy/src/node_daemon/remote_setup/payload.rs");
    assert_contains(
        &app_payload,
        "build_remote_setup_payload(RemoteSetupPayloadInput",
        "app remote setup payload code should adapt into deploy crate plans",
    );
}

#[test]
fn runtime_reports_and_proxy_dispatch_use_typed_adapters() {
    let transport_spx = read_repo_file("crates/ssh-proxy-transport/src/spx.rs");
    assert_contains(
        &transport_spx,
        "pub struct SpxBridgeWorkerSnapshot",
        "transport crate should own SPX worker report DTOs",
    );

    let controller_status = read_repo_file("crates/ssh-proxy/src/controller/status.rs");
    assert_contains(
        &controller_status,
        "ssh_proxy_transport::spx::SpxBridgeWorkerSnapshot",
        "controller status should render through the transport DTO",
    );

    let socks_main = read_repo_file("crates/ssh-proxy/src/socks.rs");
    let socks_tunnel = read_repo_file("crates/ssh-proxy/src/socks/tunnel.rs");
    assert_contains(
        &socks_main,
        "mod tunnel;",
        "SOCKS entrypoint should delegate tunnel backend dispatch",
    );
    assert_contains(
        &socks_tunnel,
        "pub(super) enum TunnelBackend",
        "SOCKS tunnel module should centralize backend selection",
    );
    assert_not_contains(
        &socks_main,
        "handle_http_proxy_ssh_native",
        "SOCKS entrypoint should not keep duplicate HTTP proxy handlers",
    );
    assert_not_contains(
        &socks_main,
        "handle_http_proxy_quic_native",
        "SOCKS entrypoint should not keep duplicate HTTP proxy handlers",
    );
}

#[test]
fn normal_source_paths_do_not_reintroduce_shell_tcp_probes() {
    let mut violations = Vec::new();
    let forbidden = [
        "curl ", "curl.exe", "netcat", "/dev/tcp", "python -", "python3 ",
    ];
    let crates_dir = workspace_root().join("crates");
    for entry in fs::read_dir(&crates_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", crates_dir.display()))
    {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                crates_dir.display()
            )
        });
        let src = entry.path().join("src");
        if src.is_dir() {
            collect_source_pattern_violations(&src, &forbidden, &mut violations);
        }
    }
    assert!(
        violations.is_empty(),
        "normal source paths should not depend on curl/nc/python shell TCP probes:\n{}",
        violations.join("\n")
    );
}

fn read_repo_file(relative: &str) -> String {
    let path = workspace_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn read_package_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("ssh_proxy package should live under crates/ssh-proxy")
        .to_path_buf()
}

fn assert_contains(haystack: &str, needle: &str, message: &str) {
    assert!(haystack.contains(needle), "{message}: missing `{needle}`");
}

fn assert_not_contains(haystack: &str, needle: &str, message: &str) {
    assert!(
        !haystack.contains(needle),
        "{message}: unexpected `{needle}`"
    );
}

fn manifest_has_direct_crate(manifest: &str, name: &str) -> bool {
    let simple = format!("{name} =");
    let quoted = format!("\"{name}\" =");
    manifest.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with(&simple) || trimmed.starts_with(&quoted)
    })
}

fn assert_manifest_avoids(relative: &str, forbidden: &[&str]) {
    let manifest = read_repo_file(relative);
    for name in forbidden {
        assert!(
            !manifest_has_production_crate(&manifest, name),
            "{relative} should not have production dependency `{name}`"
        );
    }
}

fn assert_source_avoids(relative: &str, forbidden: &[&str]) {
    let mut violations = Vec::new();
    collect_source_pattern_violations(&workspace_root().join(relative), forbidden, &mut violations);
    assert!(
        violations.is_empty(),
        "{relative} should not import across crate layers:\n{}",
        violations.join("\n")
    );
}

fn manifest_has_production_crate(manifest: &str, name: &str) -> bool {
    let mut in_production_dependencies = false;
    manifest.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_production_dependencies = trimmed == "[dependencies]"
                || (trimmed.starts_with("[target.")
                    && trimmed.ends_with(".dependencies]")
                    && !trimmed.ends_with(".dev-dependencies]"));
            return false;
        }
        in_production_dependencies && dependency_line_matches(trimmed, name)
    })
}

fn dependency_line_matches(trimmed: &str, name: &str) -> bool {
    let simple = format!("{name} =");
    let workspace = format!("{name}.workspace");
    let quoted = format!("\"{name}\" =");
    let quoted_workspace = format!("\"{name}\".workspace");
    trimmed.starts_with(&simple)
        || trimmed.starts_with(&workspace)
        || trimmed.starts_with(&quoted)
        || trimmed.starts_with(&quoted_workspace)
}

fn collect_source_pattern_violations(
    path: &Path,
    forbidden: &[&str],
    violations: &mut Vec<String>,
) {
    let entries =
        fs::read_dir(path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                path.display()
            )
        });
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_source_pattern_violations(&entry_path, forbidden, violations);
            continue;
        }
        if entry_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let text = fs::read_to_string(&entry_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", entry_path.display()));
        for pattern in forbidden {
            if text.contains(pattern) {
                violations.push(format!("{} contains `{pattern}`", entry_path.display()));
            }
        }
    }
}

fn collect_workspace_source_ffi_violations(violations: &mut Vec<String>) {
    let crates_dir = workspace_root().join("crates");
    let entries = fs::read_dir(&crates_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", crates_dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                crates_dir.display()
            )
        });
        let src_dir = entry.path().join("src");
        if src_dir.is_dir() {
            collect_rust_source_ffi_violations(&src_dir, violations);
        }
    }
}

fn collect_rust_source_ffi_violations(path: &Path, violations: &mut Vec<String>) {
    let entries =
        fs::read_dir(path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                path.display()
            )
        });
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_rust_source_ffi_violations(&entry_path, violations);
            continue;
        }
        if entry_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let text = fs::read_to_string(&entry_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", entry_path.display()));
        for pattern in ["extern \"C\"", "#[link("] {
            if text.contains(pattern) {
                violations.push(format!("{} contains `{pattern}`", entry_path.display()));
            }
        }
    }
}
