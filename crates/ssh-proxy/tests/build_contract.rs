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
fn remote_e2e_harness_stays_opt_in_and_sanitized() {
    let entry = read_repo_file("crates/ssh-proxy/tests/remote_e2e.rs");
    let support = [
        read_repo_file("crates/ssh-proxy/tests/support/remote_e2e/mod.rs"),
        read_repo_file("crates/ssh-proxy/tests/support/remote_e2e/config.rs"),
        read_repo_file("crates/ssh-proxy/tests/support/remote_e2e/command.rs"),
        read_repo_file("crates/ssh-proxy/tests/support/remote_e2e/sandbox.rs"),
    ]
    .join("\n");
    let example = read_repo_file("scripts/remote-e2e.local.example.ps1");

    for test_name in ["remote_probe", "remote_smoke", "remote_full"] {
        assert_contains(
            &entry,
            &format!("fn {test_name}()"),
            "remote e2e harness should expose the documented ignored test entry",
        );
    }
    assert!(
        entry.matches("#[ignore]").count() >= 3,
        "remote e2e tests should stay ignored and out of default gates"
    );

    for env_name in [
        "SSH_PROXY_REMOTE_E2E",
        "SSH_PROXY_REMOTE_LEVEL",
        "SSH_PROXY_REMOTE_TARGETS",
        "SSH_PROXY_REMOTE_JUMP_TARGET",
        "SSH_PROXY_REMOTE_DIRECT_TARGET",
        "SSH_PROXY_REMOTE_UPSTREAM_PROXY",
        "SSH_PROXY_REMOTE_ACCEPT_NEW",
        "SSH_PROXY_REMOTE_KEEP",
    ] {
        assert_contains(
            &support,
            env_name,
            "remote e2e harness should be configured only by environment",
        );
        assert_contains(
            &example,
            env_name,
            "remote e2e example should document every public environment knob",
        );
    }

    assert_contains(
        &support,
        "remote_cleanup",
        "remote e2e harness should clean up temporary daemon state by default",
    );
    assert_contains(
        &support,
        "/tmp/ssh_proxy-e2e-",
        "remote e2e harness should isolate remote temporary files",
    );
    assert_not_contains(
        &support,
        "102",
        "remote e2e harness should not bake private target aliases into code",
    );
    assert_not_contains(
        &support,
        "125",
        "remote e2e harness should not bake private target aliases into code",
    );
    assert_not_contains(
        &example,
        "102",
        "remote e2e example should use placeholder aliases",
    );
    assert_not_contains(
        &example,
        "125",
        "remote e2e example should use placeholder aliases",
    );
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
fn workspace_member_dependencies_use_workspace_table() {
    let root = read_repo_file("Cargo.toml");
    let local_crates = local_workspace_crate_names();
    for name in &local_crates {
        assert_contains(
            &root,
            &format!("{name} = {{ path = \"crates/{name}\" }}"),
            "root workspace dependencies should list every internal crate once",
        );
    }

    let mut violations = Vec::new();
    for manifest_path in workspace_member_manifests() {
        let relative = relative_workspace_path(&manifest_path);
        let text = fs::read_to_string(&manifest_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", manifest_path.display()));
        let mut in_dependency_section = false;
        for (line_number, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_dependency_section = trimmed.contains("dependencies");
                continue;
            }
            if !in_dependency_section {
                continue;
            }
            for name in &local_crates {
                if dependency_line_matches(trimmed, name) && !trimmed.contains(".workspace = true")
                {
                    violations.push(format!(
                        "{relative}:{} uses `{}` without `.workspace = true`",
                        line_number + 1,
                        name
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "workspace members should depend on internal crates through [workspace.dependencies]:\n{}",
        violations.join("\n")
    );
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
fn route_and_proxy_session_policy_live_outside_app_runtime() {
    let app_route_conflict =
        read_repo_file("crates/ssh-proxy/src/node_daemon/routes/conflict_policy.rs");
    assert_contains(
        &app_route_conflict,
        "decide_route_conflict",
        "app route conflict code should delegate pure decisions to route crate",
    );
    assert_not_contains(
        &app_route_conflict,
        "crate::cli",
        "route conflict policy should not depend on CLI types",
    );

    let route_conflict = read_repo_file("crates/ssh-proxy-route/src/conflict.rs");
    for symbol in [
        "pub struct RouteConflictInput",
        "pub enum RouteConflictDecision",
        "pub fn decide_route_conflict",
        "pub fn route_specs_match_values",
    ] {
        assert_contains(
            &route_conflict,
            symbol,
            "route crate should own pure route conflict semantics",
        );
    }

    let route_preflight = read_repo_file("crates/ssh-proxy-route/src/preflight.rs");
    for symbol in [
        "pub struct RouteProbeResult",
        "pub struct RoutePreflightInput",
        "pub struct RoutePreflightDecision",
        "pub struct RouteFallbackInput",
        "pub struct RouteFallbackDecision",
        "pub fn decide_route_preflight",
        "pub fn decide_route_fallback",
    ] {
        assert_contains(
            &route_preflight,
            symbol,
            "route crate should own preflight probe classification and fallback policy",
        );
    }
    let app_preflight = read_repo_file("crates/ssh-proxy/src/route/preflight.rs");
    assert_contains(
        &app_preflight,
        "decide_route_preflight",
        "app preflight should delegate pure classification to route crate",
    );
    assert_contains(
        &app_preflight,
        "decide_route_fallback",
        "app preflight should delegate fallback selection to route crate",
    );
    let app_response = read_repo_file("crates/ssh-proxy/src/route/response.rs");
    assert_not_contains(
        &app_response,
        "fn candidate_failures",
        "app route response should not retain preflight candidate classification logic",
    );

    let route_remote_use = read_repo_file("crates/ssh-proxy-route/src/remote_use.rs");
    for symbol in [
        "pub enum RemoteUseConnectMode",
        "pub enum RemoteUsePlan",
        "pub struct RemoteUseDecision",
        "pub struct RemoteUseInput",
        "pub fn decide_remote_use",
        "pub fn resolve_remote_use_local_peer",
    ] {
        assert_contains(
            &route_remote_use,
            symbol,
            "route crate should own remote-uses-local direct/reverse-link policy",
        );
    }
    let app_selection = read_repo_file("crates/ssh-proxy/src/route/selection.rs");
    assert_contains(
        &app_selection,
        "RemoteUseInput",
        "app route selection should adapt CLI/config into route remote-use input",
    );
    assert_not_contains(
        &app_selection,
        "enum RemoteUsePlan",
        "app route selection should not retain remote-use plan enums",
    );
    assert_not_contains(
        &app_selection,
        "struct RemoteUseDecision",
        "app route selection should not retain remote-use decision DTOs",
    );

    let daemon_spec = read_repo_file("crates/ssh-proxy-daemon/src/session_spec.rs");
    for symbol in [
        "pub struct ProxySessionSpec",
        "pub struct SshTargetSpec",
        "pub struct RemotePortPolicy",
        "pub struct ApplyPolicy",
        "pub fn sanitize_key",
        "pub fn proxy_url_for_remote",
        "pub fn proxy_session_specs_match",
    ] {
        assert_contains(
            &daemon_spec,
            symbol,
            "daemon crate should own pure proxy session spec semantics",
        );
    }

    let app_spec = read_repo_file("crates/ssh-proxy/src/node_daemon/proxy_session/spec.rs");
    assert_contains(
        &app_spec,
        "proxy_session_spec_from_up_args",
        "app proxy session spec module should only adapt CLI args into daemon specs",
    );
    assert_contains(
        &app_spec,
        "crate::cli",
        "CLI dependency should stay in the app adapter",
    );
    let app_proxy_session = read_repo_file("crates/ssh-proxy/src/node_daemon/proxy_session.rs");
    assert_not_contains(
        &app_proxy_session,
        "fn proxy_session_specs_match",
        "app proxy session runtime should not retain pure spec matching logic",
    );
}

#[test]
fn service_status_summaries_live_in_service_crate() {
    let service_status = read_repo_file("crates/ssh-proxy-service/src/status.rs");
    for symbol in [
        "pub struct ServiceManagerSummaryInput",
        "pub fn service_manager_summary",
        "pub fn selected_control_summary",
        "pub fn service_candidates_summary",
        "pub fn service_state_name",
        "pub fn service_next_action",
        "pub fn persistent_manager_kind",
        "pub fn control_endpoint_kind_from_str",
    ] {
        assert_contains(
            &service_status,
            symbol,
            "service crate should own status summary DTO rendering",
        );
    }

    let app_status = read_repo_file("crates/ssh-proxy/src/service/status.rs");
    for delegated in [
        "ssh_proxy_service::service_manager_summary",
        "ssh_proxy_service::selected_control_summary",
        "ssh_proxy_service::service_candidates_summary",
        "ssh_proxy_service::service_state_name",
        "ssh_proxy_service::service_next_action",
    ] {
        assert_contains(
            &app_status,
            delegated,
            "app service status should delegate pure report rendering to service crate",
        );
    }
    for local_logic in [
        "fn persistent_manager_kind",
        "fn control_endpoint_kind_from_str",
        "let mut candidates = Vec::new()",
        "\"session_daemon_fallback\": {",
    ] {
        assert_not_contains(
            &app_status,
            local_logic,
            "app service status should not retain moved service summary logic",
        );
    }
}

#[test]
fn service_health_and_peer_compatibility_live_in_service_crate() {
    let service_health = read_repo_file("crates/ssh-proxy-service/src/health.rs");
    for symbol in [
        "pub struct ConfigFileHealthInput",
        "pub enum ConfigFileHealthState",
        "pub struct RouteStoreHealthInput",
        "pub enum RouteStoreHealthState",
        "pub struct BinaryHealthInput",
        "pub enum EndpointHealthInput",
        "pub struct PeerHealthInput",
        "pub struct PeerCompatibilityInput",
        "pub struct PeerVersionCheckInput",
        "pub fn service_health_report",
        "pub fn config_file_health_report",
        "pub fn route_store_health_report",
        "pub fn binary_health_report",
        "pub fn endpoint_health_report",
        "pub fn peer_health_report",
        "pub fn peer_compatibility_report",
        "pub fn peer_version_check_report",
        "protocol_compatibility_report",
        "compare_dotted_versions",
        "PEER_VERSION",
        "default_features",
    ] {
        assert_contains(
            &service_health,
            symbol,
            "service crate should own service health DTOs and peer compatibility semantics",
        );
    }

    let app_health = read_repo_file("crates/ssh-proxy/src/service/health.rs");
    for delegated in [
        "ssh_proxy_service::{",
        "service_health_report(ServiceHealthInput",
        "config_file_health_report(ConfigFileHealthInput",
        "route_store_health_report(RouteStoreHealthInput",
        "binary_health_report(BinaryHealthInput",
        "endpoint_health_report(EndpointHealthInput",
        "peer_compatibility_report(PeerCompatibilityInput",
        "peer_health_report(PeerHealthInput",
        "peer_registry_health_report(PeerRegistryHealthInput",
    ] {
        assert_contains(
            &app_health,
            delegated,
            "app service health should adapt runtime probes into service crate reports",
        );
    }
    for local_logic in [
        "json!",
        "fn duplicate_route_ids",
        "protocol_compatibility_report",
        "compare_dotted_versions",
    ] {
        assert_not_contains(
            &app_health,
            local_logic,
            "app service health should not retain moved health report logic",
        );
    }

    let app_peer_health_path = workspace_root().join("crates/ssh-proxy/src/service/peer_health.rs");
    assert!(
        !app_peer_health_path.exists(),
        "app service peer health helper should be collapsed into service crate DTOs"
    );

    let app_peer_compat = read_repo_file("crates/ssh-proxy/src/node_daemon/peers/compatibility.rs");
    for delegated in [
        "peer_version_check_report(PeerVersionCheckInput",
        "unrecorded_peer_version_check_report",
        "PeerCompatibilityInput",
    ] {
        assert_contains(
            &app_peer_compat,
            delegated,
            "app peer compatibility should adapt descriptors into service crate reports",
        );
    }
    for local_logic in [
        "protocol_compatibility_report",
        "compare_dotted_versions",
        "peer_transport::PEER_VERSION",
        "fn binary_version_check",
        "fn version_next_action",
        "fn version_status",
    ] {
        assert_not_contains(
            &app_peer_compat,
            local_logic,
            "app peer compatibility should not retain protocol compatibility policy",
        );
    }
}

#[test]
fn lifecycle_provider_contracts_live_in_lifecycle_crate() {
    let app_provider = read_repo_file("crates/ssh-proxy/src/peer_lifecycle/service_provider.rs");
    assert_contains(
        &app_provider,
        "ssh_proxy_lifecycle::service_provider::{PeerServiceProvider, ServiceProviderPlan}",
        "app peer lifecycle provider module should re-export lifecycle provider contracts",
    );
    assert_not_contains(
        &app_provider,
        "mod contract;",
        "app peer lifecycle provider module should not keep duplicate provider contracts",
    );

    let app_contract_path =
        workspace_root().join("crates/ssh-proxy/src/peer_lifecycle/service_provider/contract.rs");
    assert!(
        !app_contract_path.exists(),
        "app peer lifecycle provider contract implementation should be removed"
    );

    let app_plans = read_repo_file("crates/ssh-proxy/src/peer_lifecycle/service_provider/plans.rs");
    assert_contains(
        &app_plans,
        "remote_service_action_plan",
        "app remote service install plan should adapt into lifecycle provider plans",
    );
    assert_contains(
        &app_plans,
        "reported_service_manager",
        "app adapter may preserve legacy reported_service_manager compatibility",
    );
    assert_not_contains(
        &app_plans,
        "pub(crate) struct ServiceProviderPlan",
        "app remote service plan adapter should not reimplement provider contracts",
    );
    assert_not_contains(
        &app_plans,
        "pub(crate) trait PeerServiceProvider",
        "app remote service plan adapter should not reimplement provider traits",
    );

    let lifecycle_contract =
        read_repo_file("crates/ssh-proxy-lifecycle/src/service_provider/contract.rs");
    for symbol in [
        "pub struct ServiceProviderPlan",
        "pub trait PeerServiceProvider",
        "impl PeerServiceProvider for ServiceProviderPlan",
    ] {
        assert_contains(
            &lifecycle_contract,
            symbol,
            "lifecycle crate should own provider contract implementations",
        );
    }

    let lifecycle_selection =
        read_repo_file("crates/ssh-proxy-lifecycle/src/service_provider/selection.rs");
    assert_contains(
        &lifecycle_selection,
        "pub fn provider_external_action_report",
        "lifecycle crate should classify provider fallback external actions",
    );
    assert_contains(
        &lifecycle_selection,
        "ExternalActionReport::fallback_provider",
        "lifecycle provider fallback should use shared external action reports",
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

    let deploy_scripts = read_repo_file("crates/ssh-proxy-deploy/src/remote_setup_scripts.rs");
    for symbol in [
        "pub fn build_git_config_script",
        "pub fn build_cleanup_script_with_git",
        "pub fn build_server_env_setup_content",
    ] {
        assert_contains(
            &deploy_scripts,
            symbol,
            "deploy crate should own pure remote setup script rendering",
        );
    }

    let remote_setup_executor =
        read_repo_file("crates/ssh-proxy/src/node_daemon/remote_setup/executor.rs");
    assert_contains(
        &remote_setup_executor,
        "RemoteSetupScriptIntent::fallback_shell",
        "app remote setup executor should classify shell fallback scripts",
    );
    assert_contains(
        &remote_setup_executor,
        "intent.class.as_str()",
        "fallback script failures should carry the fallback classification",
    );
    assert_contains(
        &remote_setup_executor,
        "intent.external_action_report()",
        "fallback script execution should expose shared external action details",
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

#[test]
fn production_command_execution_stays_in_execution_crates() {
    let mut violations = Vec::new();
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
            collect_command_execution_violations(&src, &mut violations);
        }
    }
    assert!(
        violations.is_empty(),
        "production Command::new should stay behind platform or lifecycle executors:\n{}",
        violations.join("\n")
    );
}

#[test]
fn production_service_paths_report_operability_errors_without_panics() {
    let windows_runner =
        read_repo_file("crates/ssh-proxy/src/node_daemon/windows_service_runner.rs");
    for forbidden in [
        "expect(\"service context mutex poisoned\")",
        "expect(\"stop sender mutex poisoned\")",
    ] {
        assert_not_contains(
            &windows_runner,
            forbidden,
            "Windows service runtime should map mutex poisoning to errors or logs",
        );
    }
    for required in [
        "fn lock_service_context",
        "Windows service context mutex poisoned",
        "Windows service stop sender mutex poisoned",
        "ServiceControlHandlerResult::Other(ERROR_GEN_FAILURE)",
    ] {
        assert_contains(
            &windows_runner,
            required,
            "Windows service runtime should expose structured operability failures",
        );
    }

    let service_plan = read_repo_file("crates/ssh-proxy/src/service/plan.rs");
    assert_not_contains(
        &service_plan,
        "last_error.expect(\"copy loop should record an error\")",
        "Windows binary copy retry should return an explicit error instead of panicking",
    );
    assert_contains(
        &service_plan,
        "copy retry loop did not run",
        "Windows binary copy retry should preserve a diagnosable impossible-state error",
    );
}

#[test]
fn production_runtime_paths_avoid_unclassified_panics() {
    let mut violations = Vec::new();
    for relative in [
        "crates/ssh-proxy/src",
        "crates/ssh-proxy-service/src",
        "crates/ssh-proxy-platform/src",
        "crates/ssh-proxy-deploy/src",
        "crates/ssh-proxy-lifecycle/src",
    ] {
        collect_production_panic_violations(&workspace_root().join(relative), &mut violations);
    }

    assert!(
        violations.is_empty(),
        "production runtime paths should return structured errors instead of panicking:\n{}",
        violations.join("\n")
    );
}

#[test]
fn runtime_operability_logs_keep_correlation_fields() {
    let routes = read_repo_file("crates/ssh-proxy/src/node_daemon/routes.rs");
    for field in [
        "route_id = %id",
        "peer = %peer",
        "execution_backend = %execution_backend",
        "fallback_used",
    ] {
        assert_contains(
            &routes,
            field,
            "route runtime logs should expose route correlation and fallback fields",
        );
    }

    let proxy_session = read_repo_file("crates/ssh-proxy/src/node_daemon/proxy_session.rs");
    for field in [
        "job_id = %job_id",
        "session_id = %session_id",
        "route_id = %route_id",
        "peer = %spec.target",
    ] {
        assert_contains(
            &proxy_session,
            field,
            "proxy session logs should expose daemon job/session/route correlation fields",
        );
    }

    let remote_setup = read_repo_file("crates/ssh-proxy/src/node_daemon/remote_setup/executor.rs");
    for field in [
        "job_id = %spec.job_id()",
        "session_id = %spec.session_id()",
        "route_id = %spec.route_id()",
        "execution_backend = \"remote_shell_bootstrap\"",
        "fallback_used = true",
    ] {
        assert_contains(
            &remote_setup,
            field,
            "remote setup fallback logs should expose correlation and fallback fields",
        );
    }

    let systemd = read_repo_file("crates/ssh-proxy/src/service/platform/systemd.rs");
    for field in [
        "execution_backend = \"provider_command\"",
        "fallback_used = true",
        "native_backend = \"systemd_dbus\"",
    ] {
        assert_contains(
            &systemd,
            field,
            "local service provider fallback logs should expose backend semantics",
        );
    }
}

#[test]
fn native_provider_success_paths_are_preferred() {
    let core_external = read_repo_file("crates/ssh-proxy-core/src/external.rs");
    for symbol in [
        "pub struct ExternalActionReport",
        "pub fn required_provider",
        "pub fn fallback_provider",
        "pub fn with_repair_action",
        "pub fn to_json",
    ] {
        assert_contains(
            &core_external,
            symbol,
            "core crate should own shared external action reports",
        );
    }

    let platform = read_repo_file("crates/ssh-proxy-platform/src/lib.rs");
    assert_contains(
        &platform,
        "pub enum ExecutionBackend",
        "platform crate should classify native and fallback execution backends",
    );
    assert_contains(
        &platform,
        "NativeApi",
        "platform backend classification should include native APIs",
    );
    assert_contains(
        &platform,
        "OwnBinary",
        "platform backend classification should include own-binary helpers",
    );
    assert_contains(
        &platform,
        "pub external_action: Value",
        "native provider outcomes should expose external action details",
    );
    assert_contains(
        &platform,
        "ExternalActionReport",
        "native provider outcomes should use the shared external action report",
    );
    assert_contains(
        &platform,
        "external_action_report",
        "platform command plans should expose shared external action details",
    );

    let systemd = read_repo_file("crates/ssh-proxy/src/service/platform/systemd.rs");
    assert_contains(
        &systemd,
        "run_systemd_plan",
        "systemd service path should try D-Bus plans before systemctl fallback",
    );

    let windows_tasks = read_repo_file("crates/ssh-proxy-platform/src/windows_tasks.rs");
    assert_contains(
        &windows_tasks,
        "Task Scheduler COM",
        "Windows scheduled task provider should use COM as the native path",
    );
    let windows_tasks_production = windows_tasks.split("#[cfg(test)]").next().unwrap_or("");
    assert_not_contains(
        windows_tasks_production,
        "schtasks",
        "platform scheduled task provider should not shell out to schtasks",
    );

    let launchd = read_repo_file("crates/ssh-proxy/src/service/platform/launchd.rs");
    let launchd_production = launchd.split("#[cfg(test)]").next().unwrap_or("");
    assert_contains(
        launchd_production,
        "daemon_program_arguments",
        "launchd plist should render tokenized ProgramArguments",
    );
    assert_not_contains(
        launchd_production,
        "<string>/bin/sh</string>",
        "launchd plist should not start the daemon through a shell",
    );
    assert_not_contains(
        launchd_production,
        "<string>-lc</string>",
        "launchd plist should not start the daemon through a shell",
    );

    let helper = read_repo_file("crates/ssh-proxy/src/deploy/helper.rs");
    assert_contains(
        &helper,
        "remote_helper_checksum_via_admin",
        "remote helper upload should try own-binary checksum first",
    );
    assert_contains(
        &helper,
        "falling back to shell checksum probe",
        "legacy checksum shell should be explicit fallback only",
    );

    let remote_setup = read_repo_file("crates/ssh-proxy/src/node_daemon/remote_setup/executor.rs");
    assert_contains(
        &remote_setup,
        "RemoteAdminIntent::GitApply",
        "remote setup should try own-binary Git config edits first",
    );
    assert_contains(
        &remote_setup,
        "falling back to script",
        "remote setup shell scripts should be explicit fallback only",
    );
}

#[test]
fn remote_admin_success_paths_use_own_binary_contract() {
    let deploy_admin = read_repo_file("crates/ssh-proxy-deploy/src/remote_admin.rs");
    for intent in [
        "Checksum",
        "Defaults",
        "Status",
        "Doctor",
        "GitApply",
        "GitCleanup",
    ] {
        assert_contains(
            &deploy_admin,
            intent,
            "deploy crate should define every remote admin intent",
        );
    }
    for field in [
        "\"execution_backend\": \"own_binary\"",
        "\"fallback_used\": false",
        "\"external_action\": remote_admin_external_action(kind)",
        "ExternalActionReport::required_provider",
    ] {
        assert_contains(
            &deploy_admin,
            field,
            "remote admin responses should expose own-binary execution semantics",
        );
    }

    let app_admin = read_repo_file("crates/ssh-proxy/src/remote/admin.rs");
    for arm in [
        "RemoteAdminIntent::Checksum",
        "RemoteAdminIntent::Defaults",
        "RemoteAdminIntent::Status",
        "RemoteAdminIntent::Doctor",
        "RemoteAdminIntent::GitApply",
        "RemoteAdminIntent::GitCleanup",
    ] {
        assert_contains(
            &app_admin,
            arm,
            "app remote admin command should handle every deploy intent",
        );
    }
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

fn workspace_member_manifests() -> Vec<std::path::PathBuf> {
    let crates_dir = workspace_root().join("crates");
    let mut manifests = Vec::new();
    for entry in fs::read_dir(&crates_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", crates_dir.display()))
    {
        let entry = entry.unwrap_or_else(|err| {
            panic!(
                "failed to read directory entry under {}: {err}",
                crates_dir.display()
            )
        });
        let manifest = entry.path().join("Cargo.toml");
        if manifest.is_file() {
            manifests.push(manifest);
        }
    }
    manifests.sort();
    manifests
}

fn local_workspace_crate_names() -> Vec<String> {
    workspace_member_manifests()
        .into_iter()
        .filter_map(|manifest| {
            let dir_name = manifest.parent()?.file_name()?.to_str()?;
            (dir_name.starts_with("ssh-proxy-")).then(|| dir_name.to_string())
        })
        .collect()
}

fn relative_workspace_path(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .unwrap_or(path)
        .display()
        .to_string()
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

    let service_broker = read_repo_file("crates/ssh-proxy-service/src/broker.rs");
    for symbol in [
        "pub struct ServiceBrokerReportInput",
        "pub fn service_broker_report",
        "\"tcp_legacy\"",
        "\"session_daemon\"",
        "\"arbitrary_shell\": false",
    ] {
        assert_contains(
            &service_broker,
            symbol,
            "service crate should own broker fallback report rendering",
        );
    }

    let app_broker = read_repo_file("crates/ssh-proxy/src/service/broker.rs");
    assert_contains(
        &app_broker,
        "ssh_proxy_service::service_broker_report",
        "app service broker should delegate pure report rendering to service crate",
    );
    for local_logic in [
        "fn broker_candidates",
        "fn select_broker_candidate",
        "fn permission_boundary",
        "fn broker_next_action",
    ] {
        assert_not_contains(
            &app_broker,
            local_logic,
            "app service broker should not retain moved broker summary logic",
        );
    }
}

fn collect_command_execution_violations(path: &Path, violations: &mut Vec<String>) {
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
            collect_command_execution_violations(&entry_path, violations);
            continue;
        }
        if entry_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if command_execution_path_is_allowed(&entry_path) {
            continue;
        }

        let text = fs::read_to_string(&entry_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", entry_path.display()));
        for pattern in ["Command::new(", "process::Command"] {
            let contains = if pattern == "Command::new(" {
                text.lines().any(contains_process_command_new)
            } else {
                text.contains(pattern)
            };
            if contains {
                violations.push(format!(
                    "{} contains `{pattern}`",
                    relative_workspace_path(&entry_path)
                ));
            }
        }
    }
}

fn collect_production_panic_violations(path: &Path, violations: &mut Vec<String>) {
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
            collect_production_panic_violations(&entry_path, violations);
            continue;
        }
        if entry_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if production_panic_path_is_allowed(&entry_path) {
            continue;
        }

        let text = fs::read_to_string(&entry_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", entry_path.display()));
        let production_text = text.split("#[cfg(test)]").next().unwrap_or("");
        for (line_number, line) in production_text.lines().enumerate() {
            for pattern in ["unwrap(", "expect(", "panic!(", "todo!(", "unimplemented!("] {
                if line.contains(pattern) {
                    violations.push(format!(
                        "{}:{} contains `{pattern}`",
                        relative_workspace_path(&entry_path),
                        line_number + 1
                    ));
                }
            }
        }
    }
}

fn command_execution_path_is_allowed(path: &Path) -> bool {
    let relative = relative_workspace_path(path).replace('\\', "/");
    relative.starts_with("crates/ssh-proxy-platform/src/")
        || relative == "crates/ssh-proxy-lifecycle/src/executor/local.rs"
        || relative.ends_with("/tests.rs")
        || relative.contains("/tests/")
}

fn production_panic_path_is_allowed(path: &Path) -> bool {
    let relative = relative_workspace_path(path).replace('\\', "/");
    relative.ends_with("/tests.rs")
        || relative.contains("/tests/")
        || relative == "crates/ssh-proxy-lifecycle/src/executor/fake.rs"
}

fn contains_process_command_new(line: &str) -> bool {
    let Some(index) = line.find("Command::new(") else {
        return false;
    };
    line[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
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
