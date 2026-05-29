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
            !manifest_has_direct_crate(&manifest, name),
            "{relative} should not depend on `{name}`"
        );
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
