use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::paths;

pub fn config_path() -> Result<PathBuf> {
    paths::config_path()
}

pub fn routes_path() -> Result<PathBuf> {
    paths::routes_path()
}

pub fn jobs_path() -> Result<PathBuf> {
    paths::jobs_path()
}

pub fn daemon_state_path() -> Result<PathBuf> {
    paths::daemon_state_path()
}

pub fn sessions_path() -> Result<PathBuf> {
    paths::sessions_path()
}

pub fn peers_path() -> Result<PathBuf> {
    paths::peers_path()
}

pub fn certs_dir() -> Result<PathBuf> {
    paths::certs_dir()
}

pub fn file_sha256_fingerprint(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(&bytes);
    let mut out = String::with_capacity("sha256:".len() + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").ok()?;
    }
    Some(out)
}

pub fn save_text_file_private(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let temp = temp_save_path(path);
    {
        let mut file = create_private_file(&temp)
            .with_context(|| format!("failed to create temp file {}", temp.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("failed to write temp file {}", temp.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync temp file {}", temp.display()))?;
    }
    replace_file(&temp, path).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            temp.display()
        )
    })?;
    Ok(())
}

fn temp_save_path(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    path.with_file_name(format!(".{name}.{pid}.{stamp}.tmp"))
}

fn create_private_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        Ok(OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?)
    }
    #[cfg(not(unix))]
    {
        Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
    }
}

fn replace_file(temp: &Path, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        if target.exists() {
            fs::remove_file(target)
                .with_context(|| format!("failed to remove {}", target.display()))?;
        }
    }
    fs::rename(temp, target).with_context(|| {
        format!(
            "failed to rename {} to {}",
            temp.display(),
            target.display()
        )
    })?;
    set_file_private(target, true)?;
    Ok(())
}

pub(super) fn set_file_private(path: &Path, secret: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if secret { 0o600 } else { 0o644 };
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, secret);
    }
    Ok(())
}
