use std::{fs, path::PathBuf};

use anyhow::{Context, Result};

const LINUX_MUSL_SIDECAR: &[u8] = include_bytes!(env!("SSH_PROXY_LINUX_MUSL_SIDECAR"));
const SIDECAR_PRESENT: &str = env!("SSH_PROXY_LINUX_MUSL_SIDECAR_PRESENT");
const SIDECAR_SHA256: &str = env!("SSH_PROXY_LINUX_MUSL_SIDECAR_SHA256");
const SIDECAR_BYTES: &str = env!("SSH_PROXY_LINUX_MUSL_SIDECAR_BYTES");

pub fn linux_musl_present() -> bool {
    SIDECAR_PRESENT == "1" && !LINUX_MUSL_SIDECAR.is_empty()
}

pub fn materialize_linux_musl() -> Result<Option<PathBuf>> {
    if !linux_musl_present() {
        return Ok(None);
    }

    let path = sidecar_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create sidecar cache dir {}", parent.display()))?;
    }
    fs::write(&path, LINUX_MUSL_SIDECAR).with_context(|| {
        format!(
            "failed to materialize linux musl sidecar {}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions)?;
    }
    Ok(Some(path))
}

fn sidecar_path() -> Result<PathBuf> {
    let mut base = dirs::cache_dir().unwrap_or_else(std::env::temp_dir);
    base.push("ssh_proxy");
    base.push(format!(
        "ssh_proxy-{}-x86_64-unknown-linux-musl",
        env!("CARGO_PKG_VERSION")
    ));
    Ok(base)
}

pub fn build_summary() -> &'static str {
    if linux_musl_present() {
        "embedded"
    } else {
        "missing"
    }
}

#[allow(dead_code)]
pub fn embedded_sha256() -> Option<&'static str> {
    linux_musl_present().then_some(SIDECAR_SHA256)
}

#[allow(dead_code)]
pub fn embedded_bytes() -> u64 {
    SIDECAR_BYTES.parse().unwrap_or(0)
}
