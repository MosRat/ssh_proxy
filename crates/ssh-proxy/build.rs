use std::{env, fs, path::PathBuf};

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest_dir.clone());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let target = env::var("TARGET").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_sidecar = out_dir.join("linux-musl-sidecar.bin");

    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("SSH_PROXY_LINUX_MUSL_BIN").map(PathBuf::from) {
        println!("cargo:rerun-if-changed={}", path.display());
        candidates.push(path);
    }
    let release_sidecar = workspace_root
        .join("target")
        .join("x86_64-unknown-linux-musl")
        .join("release")
        .join("ssh_proxy");
    println!("cargo:rerun-if-changed={}", release_sidecar.display());
    candidates.push(release_sidecar);
    if profile != "release" {
        candidates.push(
            workspace_root
                .join("target")
                .join("x86_64-unknown-linux-musl")
                .join(&profile)
                .join("ssh_proxy"),
        );
        candidates.push(
            workspace_root
                .join("target")
                .join("x86_64-unknown-linux-musl")
                .join("debug")
                .join("ssh_proxy"),
        );
    }
    candidates.push(
        workspace_root
            .join("assets")
            .join("ssh_proxy-x86_64-unknown-linux-musl"),
    );
    let sidecar = candidates.into_iter().find(|candidate| candidate.exists());

    println!("cargo:rerun-if-env-changed=SSH_PROXY_LINUX_MUSL_BIN");
    println!("cargo:rerun-if-env-changed=SSH_PROXY_ALLOW_MISSING_SIDECAR");
    println!(
        "cargo:rerun-if-changed={}",
        workspace_root
            .join("assets")
            .join("ssh_proxy-x86_64-unknown-linux-musl")
            .display()
    );

    if let Some(path) = sidecar {
        let bytes = fs::read(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read linux musl sidecar from {}: {err}",
                path.display()
            )
        });
        fs::write(&out_sidecar, &bytes).unwrap_or_else(|err| {
            panic!(
                "failed to copy linux musl sidecar from {}: {err}",
                path.display()
            )
        });
        println!("cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR_PRESENT=1");
        println!(
            "cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR_SHA256={}",
            sha256_hex(&bytes)
        );
        println!(
            "cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR_BYTES={}",
            bytes.len()
        );
    } else if target == "x86_64-unknown-linux-musl"
        || env::var_os("SSH_PROXY_ALLOW_MISSING_SIDECAR").is_some()
    {
        fs::write(&out_sidecar, []).expect("failed to write empty sidecar placeholder");
        println!("cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR_PRESENT=0");
        println!("cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR_SHA256=");
        println!("cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR_BYTES=0");
    } else {
        panic!(
            "non-Linux-musl builds must carry a Linux musl sidecar. Run `cargo zigbuild --target x86_64-unknown-linux-musl` first, or set SSH_PROXY_LINUX_MUSL_BIN to an existing helper."
        );
    }

    println!(
        "cargo:rustc-env=SSH_PROXY_LINUX_MUSL_SIDECAR={}",
        out_sidecar.display()
    );
}
