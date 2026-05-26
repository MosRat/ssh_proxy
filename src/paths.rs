use std::path::PathBuf;

use anyhow::{Result, anyhow};

const APP_DIR: &str = ".ssh_proxy";

pub fn app_home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SSH_PROXY_HOME") {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot determine home directory"))?;
    Ok(home.join(APP_DIR))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_home()?.join("config.toml"))
}

pub fn routes_path() -> Result<PathBuf> {
    Ok(app_home()?.join("routes.json"))
}

pub fn certs_dir() -> Result<PathBuf> {
    Ok(app_home()?.join("certs"))
}
