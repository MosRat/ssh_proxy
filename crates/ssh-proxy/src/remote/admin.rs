use std::{
    env,
    fs::File,
    io::{BufReader, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ssh_proxy_config::{AppConfig, default_node_name, first_available_addr, is_addr_available};
use ssh_proxy_deploy::{
    RemoteAdminChecksumReport, RemoteAdminDefaultsReport, RemoteAdminIntent,
    apply_git_proxy_config, cleanup_git_proxy_config, remote_admin_error, remote_admin_ok,
};
use tokio::io::AsyncReadExt;

pub async fn run(json_arg: Option<String>) -> Result<()> {
    let input = match json_arg {
        Some(input) => input,
        None => {
            let mut input = String::new();
            tokio::io::stdin()
                .read_to_string(&mut input)
                .await
                .context("failed to read remote admin intent from stdin")?;
            input
        }
    };
    let intent: RemoteAdminIntent =
        serde_json::from_str(&input).context("invalid remote admin intent JSON")?;
    let kind = intent.kind();
    match handle_intent(intent) {
        Ok(data) => {
            println!("{}", remote_admin_ok(kind, data));
            Ok(())
        }
        Err(err) => {
            println!("{}", remote_admin_error(kind, &err.to_string()));
            Err(err)
        }
    }
}

fn handle_intent(intent: RemoteAdminIntent) -> Result<Value> {
    match intent {
        RemoteAdminIntent::Checksum { path } => checksum(&path),
        RemoteAdminIntent::Defaults {
            preferred_transport,
            preferred_control,
        } => defaults(preferred_transport, preferred_control),
        RemoteAdminIntent::Status {
            remote_tcp,
            remote_path,
        } => Ok(status(remote_tcp, remote_path)),
        RemoteAdminIntent::Doctor {
            remote_tcp,
            remote_path,
        } => Ok(doctor(remote_tcp, remote_path)),
        RemoteAdminIntent::GitApply {
            config_path,
            workspace_path,
            http_proxy,
            https_proxy,
        } => {
            let path = resolve_git_config_path(config_path, workspace_path)?;
            let report =
                apply_git_proxy_config(&path, http_proxy.as_deref(), https_proxy.as_deref())
                    .map_err(anyhow::Error::msg)?;
            serde_json::to_value(report).context("failed to render git apply report")
        }
        RemoteAdminIntent::GitCleanup {
            config_path,
            workspace_path,
        } => {
            let path = resolve_git_config_path(config_path, workspace_path)?;
            let report = cleanup_git_proxy_config(&path).map_err(anyhow::Error::msg)?;
            serde_json::to_value(report).context("failed to render git cleanup report")
        }
    }
}

fn checksum(path: &str) -> Result<Value> {
    let path = expand_remote_path(path);
    let file = File::open(&path)
        .with_context(|| format!("failed to open remote helper {}", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("failed to stat remote helper {}", path.display()))?
        .len();
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("failed to read remote helper {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let report = RemoteAdminChecksumReport {
        path: path.display().to_string(),
        sha256: hex_lower(&hasher.finalize()),
        len,
    };
    serde_json::to_value(report).context("failed to render checksum report")
}

fn defaults(preferred_transport: SocketAddr, preferred_control: SocketAddr) -> Result<Value> {
    let config_path = peer_config_path();
    let config = read_app_config(&config_path).unwrap_or_default();
    let transport = first_available_addr(preferred_transport, 200);
    let control = first_available_addr_except(preferred_control, 200, transport);
    let report = RemoteAdminDefaultsReport {
        transport,
        control,
        node_id: config.identity.node_id,
        node_name: config.identity.node_name.unwrap_or_else(default_node_name),
        config: config_path.display().to_string(),
    };
    serde_json::to_value(report).context("failed to render defaults report")
}

fn first_available_addr_except(
    preferred: SocketAddr,
    span: u16,
    reserved: SocketAddr,
) -> SocketAddr {
    let mut candidate = first_available_addr(preferred, span);
    if candidate != reserved {
        return candidate;
    }
    let start = candidate.port().saturating_add(1);
    let ip = candidate.ip();
    for offset in 0..span {
        let Some(port) = start.checked_add(offset) else {
            break;
        };
        candidate = SocketAddr::new(ip, port);
        if candidate != reserved && is_addr_available(candidate) {
            return candidate;
        }
    }
    preferred
}

fn status(remote_tcp: Option<SocketAddr>, remote_path: Option<String>) -> Value {
    let base = ssh_proxy_home();
    json!({
        "remote_tcp": remote_tcp,
        "remote_path": remote_path,
        "config": peer_config_path().display().to_string(),
        "peer_state": read_json_file(&base.join("peer_state.json")),
        "install_report": read_json_file(&base.join("install_report.json")),
        "health": read_json_file(&base.join("health.json")),
    })
}

fn doctor(remote_tcp: Option<SocketAddr>, remote_path: Option<String>) -> Value {
    let remote_path_expanded = remote_path.as_deref().map(expand_remote_path);
    json!({
        "os": env::consts::OS,
        "arch": env::consts::ARCH,
        "family": env::consts::FAMILY,
        "pid": std::process::id(),
        "user": env::var("USER").or_else(|_| env::var("USERNAME")).ok(),
        "home": dirs::home_dir().map(|path| path.display().to_string()),
        "current_exe": env::current_exe().ok().map(|path| path.display().to_string()),
        "remote_path": remote_path,
        "remote_path_exists": remote_path_expanded.as_ref().is_some_and(|path| path.exists()),
        "status": status(remote_tcp, remote_path_expanded.map(|path| path.display().to_string())),
    })
}

fn read_app_config(path: &Path) -> Result<AppConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse config {}", path.display()))
}

fn read_json_file(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn peer_config_path() -> PathBuf {
    ssh_proxy_home().join("config.toml")
}

fn ssh_proxy_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(env::temp_dir)
        .join(".ssh_proxy")
}

fn expand_remote_path(value: &str) -> PathBuf {
    let home_expanded = if let Some(rest) = value.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(rest).display().to_string())
            .unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    };
    if cfg!(windows) {
        PathBuf::from(expand_percent_env(&home_expanded))
    } else {
        PathBuf::from(home_expanded)
    }
}

fn resolve_git_config_path(
    config_path: Option<String>,
    workspace_path: Option<String>,
) -> Result<PathBuf> {
    if let Some(config_path) = config_path {
        return Ok(expand_remote_path(&config_path));
    }
    if let Some(workspace_path) = workspace_path {
        return workspace_git_config_path(&expand_remote_path(&workspace_path));
    }
    anyhow::bail!("git admin intent requires config_path or workspace_path")
}

fn workspace_git_config_path(workspace_path: &Path) -> Result<PathBuf> {
    let mut dir = if workspace_path.is_file() {
        workspace_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| workspace_path.to_path_buf())
    } else {
        workspace_path.to_path_buf()
    };
    loop {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Ok(dot_git.join("config"));
        }
        if dot_git.is_file() {
            let text = std::fs::read_to_string(&dot_git)
                .with_context(|| format!("failed to read {}", dot_git.display()))?;
            if let Some(rest) = text.trim().strip_prefix("gitdir:") {
                let git_dir = PathBuf::from(rest.trim());
                let git_dir = if git_dir.is_absolute() {
                    git_dir
                } else {
                    dir.join(git_dir)
                };
                return Ok(git_dir.join("config"));
            }
        }
        if !dir.pop() {
            anyhow::bail!(
                "workspace path is not inside a Git worktree: {}",
                workspace_path.display()
            );
        }
    }
}

fn expand_percent_env(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let Some(end) = rest.find('%') else {
            out.push('%');
            out.push_str(rest);
            return out;
        };
        let name = &rest[..end];
        if name.is_empty() {
            out.push_str("%%");
        } else if let Ok(value) = env::var(name) {
            out.push_str(&value);
        } else {
            out.push('%');
            out.push_str(name);
            out.push('%');
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_env_expansion_keeps_unknown_values() {
        assert_eq!(
            expand_percent_env(r"%SSH_PROXY_UNKNOWN%\bin"),
            r"%SSH_PROXY_UNKNOWN%\bin"
        );
    }

    #[test]
    fn remote_admin_status_reads_optional_files() {
        let value = status(None, Some("/tmp/ssh_proxy".to_string()));

        assert_eq!(value["remote_path"], "/tmp/ssh_proxy");
        assert!(value.get("config").is_some());
    }

    #[test]
    fn remote_admin_defaults_keep_control_distinct_from_transport() {
        let reserved = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let reserved_addr = reserved.local_addr().unwrap();
        let selected = first_available_addr_except(reserved_addr, 20, reserved_addr);

        assert_ne!(selected, reserved_addr);
        assert_eq!(selected.ip(), reserved_addr.ip());
    }
}
