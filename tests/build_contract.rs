use std::{fs, path::Path};

#[test]
fn cargo_manifest_keeps_release_binary_contract() {
    let manifest = read_repo_file("Cargo.toml");

    assert_contains(
        &manifest,
        "\nmimalloc = ",
        "Cargo.toml should keep mimalloc as a direct dependency",
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
    let main_rs = read_repo_file("src/main.rs");

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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    collect_rust_source_ffi_violations(&root, &mut violations);

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
        "cargo test --tests",
        "cargo build --release",
        "cargo zigbuild --target x86_64-unknown-linux-musl --release",
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

fn read_repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
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
