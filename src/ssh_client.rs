use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use russh::{
    client::{self, DisconnectReason},
    keys,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, error, info, warn};

use crate::{cli, ssh_auth};

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub exit_status: u32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub identities: Vec<PathBuf>,
    pub known_hosts: Option<PathBuf>,
    pub accept_new: bool,
    pub insecure_ignore_host_key: bool,
    pub jumps: Vec<Target>,
    jump_specs: Vec<String>,
}

pub struct Client {
    target: Target,
    session: client::Handle<ClientHandler>,
    _parents: Vec<client::Handle<ClientHandler>>,
}

impl Client {
    pub async fn connect_proxy_args(args: &cli::ProxyArgs) -> Result<Self> {
        if args.ssh_command.is_some() {
            bail!(
                "--ssh-command cannot be executed by russh; use --ssh-arg/-F/-i/-p/--user or ~/.ssh/config"
            );
        }
        let target = resolve_target(
            &args.target,
            &args.ssh_args,
            args.user.clone(),
            args.port,
            args.identity.clone(),
            args.config.clone(),
            args.known_hosts.clone(),
            args.accept_new,
            args.insecure_ignore_host_key,
            args.jump.clone(),
        )?;
        Self::connect(target).await
    }

    pub async fn connect_install_args(args: &cli::InstallRemoteArgs) -> Result<Self> {
        if args.ssh_command.is_some() {
            bail!(
                "--ssh-command cannot be executed by russh; use --ssh-arg/-F/-i/-p/--user or ~/.ssh/config"
            );
        }
        let target = resolve_target(
            &args.target,
            &args.ssh_args,
            args.user.clone(),
            args.port,
            args.identity.clone(),
            args.config.clone(),
            args.known_hosts.clone(),
            args.accept_new,
            args.insecure_ignore_host_key,
            args.jump.clone(),
        )?;
        Self::connect(target).await
    }

    async fn connect(target: Target) -> Result<Self> {
        if target.jumps.is_empty() {
            let session = connect_tcp_session(&target)
                .await
                .with_context(|| format!("failed to connect SSH target {}", target.describe()))?;
            return Ok(Self {
                target,
                session,
                _parents: Vec::new(),
            });
        }

        let mut parents = Vec::new();
        let mut current = connect_tcp_session(&target.jumps[0])
            .await
            .with_context(|| {
                format!(
                    "failed to connect first jump {}",
                    target.jumps[0].describe()
                )
            })?;
        for hop in target.jumps.iter().skip(1) {
            let stream = open_direct_stream(&current, hop)
                .await
                .with_context(|| format!("jump channel to {} failed", hop.describe()))?;
            let next = connect_stream_session(hop, stream)
                .await
                .with_context(|| format!("failed to authenticate jump {}", hop.describe()))?;
            parents.push(current);
            current = next;
        }

        let stream = open_direct_stream(&current, &target)
            .await
            .with_context(|| {
                format!("jump channel to final target {} failed", target.describe())
            })?;
        let session = connect_stream_session(&target, stream)
            .await
            .with_context(|| {
                format!("failed to authenticate final target {}", target.describe())
            })?;
        parents.push(current);

        Ok(Self {
            target,
            session,
            _parents: parents,
        })
    }

    pub fn target(&self) -> &Target {
        &self.target
    }
}

pub fn resolve_route_target(args: &cli::RouteArgs) -> Result<Target> {
    resolve_target(
        &args.target,
        &args.ssh_args,
        args.user.clone(),
        args.ssh_port,
        args.identity.clone(),
        args.config.clone(),
        args.known_hosts.clone(),
        args.accept_new,
        args.insecure_ignore_host_key,
        args.jump.clone(),
    )
}

impl Target {
    fn describe(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }
}

async fn connect_tcp_session(target: &Target) -> Result<client::Handle<ClientHandler>> {
    let config = ssh_config();
    let addr = (target.host.as_str(), target.port);
    let handler = handler_for(target);
    info!(host = %target.host, port = target.port, user = %target.user, "connecting with russh");
    let mut session = client::connect(config, addr, handler)
        .await
        .with_context(|| format!("russh TCP connect failed for {}", target.describe()))?;
    ssh_auth::authenticate(&mut session, target).await?;
    info!(target = %target.describe(), "russh authentication succeeded");
    Ok(session)
}

async fn connect_stream_session<S>(
    target: &Target,
    stream: S,
) -> Result<client::Handle<ClientHandler>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let handler = handler_for(target);
    let mut session = client::connect_stream(ssh_config(), stream, handler)
        .await
        .with_context(|| format!("russh stream connect failed for {}", target.describe()))?;
    ssh_auth::authenticate(&mut session, target).await?;
    info!(target = %target.describe(), "russh authentication over jump succeeded");
    Ok(session)
}

async fn open_direct_stream(
    session: &client::Handle<ClientHandler>,
    target: &Target,
) -> Result<russh::ChannelStream<client::Msg>> {
    info!(target = %target.describe(), "opening russh direct-tcpip jump channel");
    let channel = session
        .channel_open_direct_tcpip(target.host.clone(), target.port as u32, "127.0.0.1", 0)
        .await
        .with_context(|| format!("direct-tcpip open failed for {}", target.describe()))?;
    Ok(channel.into_stream())
}

fn ssh_config() -> Arc<client::Config> {
    let mut config = client::Config::default();
    config.keepalive_interval = Some(Duration::from_secs(20));
    config.keepalive_max = 3;
    config.nodelay = true;
    Arc::new(config)
}

fn handler_for(target: &Target) -> ClientHandler {
    ClientHandler {
        host: target.host.clone(),
        port: target.port,
        known_hosts: target.known_hosts.clone(),
        accept_new: target.accept_new,
        insecure_ignore_host_key: target.insecure_ignore_host_key,
    }
}

impl Client {
    pub async fn exec_stream(&self, command: String) -> Result<russh::ChannelStream<client::Msg>> {
        let channel = self
            .session
            .channel_open_session()
            .await
            .context("failed to open SSH session channel")?;
        channel
            .exec(true, command.into_bytes())
            .await
            .context("failed to exec remote command")?;
        Ok(channel.into_stream())
    }

    pub async fn direct_tcpip_stream(
        &self,
        host: String,
        port: u16,
    ) -> Result<russh::ChannelStream<client::Msg>> {
        let target = Target {
            alias: host.clone(),
            host,
            port,
            user: self.target.user.clone(),
            identities: Vec::new(),
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jumps: Vec::new(),
            jump_specs: Vec::new(),
        };
        open_direct_stream(&self.session, &target).await
    }

    pub async fn exec_upload(&self, command: String, bytes: Vec<u8>) -> Result<()> {
        let mut channel = self
            .session
            .channel_open_session()
            .await
            .context("failed to open upload channel")?;
        channel
            .exec(true, command.into_bytes())
            .await
            .context("failed to exec upload command")?;
        channel
            .data_bytes(Bytes::from(bytes))
            .await
            .context("failed to send upload bytes")?;
        channel.eof().await.ok();

        let mut status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                russh::ChannelMsg::Close => break,
                russh::ChannelMsg::ExtendedData { data, .. } => {
                    if let Ok(s) = std::str::from_utf8(&data) {
                        for line in s.lines() {
                            warn!(target: "remote-stderr", "{line}");
                        }
                    }
                }
                _ => {}
            }
        }
        if status.unwrap_or(0) != 0 {
            bail!("remote upload command exited with status {:?}", status);
        }
        Ok(())
    }

    pub async fn exec_status(&self, command: String) -> Result<()> {
        let mut channel = self
            .session
            .channel_open_session()
            .await
            .context("failed to open command channel")?;
        channel.exec(true, command.into_bytes()).await?;
        channel.eof().await.ok();
        let mut status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                russh::ChannelMsg::Close => break,
                russh::ChannelMsg::Data { data } | russh::ChannelMsg::ExtendedData { data, .. } => {
                    if let Ok(s) = std::str::from_utf8(&data) {
                        for line in s.lines() {
                            info!(target: "remote", "{line}");
                        }
                    }
                }
                _ => {}
            }
        }
        if status.unwrap_or(0) != 0 {
            bail!("remote command exited with status {:?}", status);
        }
        Ok(())
    }

    pub async fn exec_output(&self, command: String) -> Result<String> {
        let output = self.exec_capture(command, None).await?;
        if output.exit_status != 0 {
            bail!(
                "remote command exited with status {}: {}",
                output.exit_status,
                output.stderr.trim()
            );
        }
        Ok(output.stdout)
    }

    pub async fn exec_capture(
        &self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> Result<ExecOutput> {
        let mut channel = self
            .session
            .channel_open_session()
            .await
            .context("failed to open command channel")?;
        channel.exec(true, command.into_bytes()).await?;
        if let Some(stdin) = stdin {
            channel
                .data_bytes(Bytes::from(stdin))
                .await
                .context("failed to send command stdin")?;
        }
        channel.eof().await.ok();
        let mut status = None;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                russh::ChannelMsg::Close => break,
                russh::ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                russh::ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                _ => {}
            }
        }
        Ok(ExecOutput {
            exit_status: status.unwrap_or(0),
            stdout: String::from_utf8(stdout).context("remote command stdout was not utf-8")?,
            stderr: String::from_utf8(stderr).context("remote command stderr was not utf-8")?,
        })
    }
}

#[derive(Clone)]
pub(crate) struct ClientHandler {
    host: String,
    port: u16,
    known_hosts: Option<PathBuf>,
    accept_new: bool,
    insecure_ignore_host_key: bool,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = std::result::Result<bool, Self::Error>> + Send {
        let host = self.host.clone();
        let port = self.port;
        let known_hosts = self.known_hosts.clone();
        let accept_new = self.accept_new;
        let insecure_ignore_host_key = self.insecure_ignore_host_key;
        let key = server_public_key.clone();
        async move {
            if insecure_ignore_host_key {
                warn!(
                    host = %host,
                    port,
                    "accepting SSH host key because --insecure-ignore-host-key is set"
                );
                return Ok(true);
            }
            if known_hosts
                .as_ref()
                .is_some_and(|path| is_null_known_hosts(path))
            {
                warn!(host = %host, port, "accepting SSH host key because known_hosts is disabled");
                return Ok(true);
            }
            let check = match known_hosts {
                Some(ref path) => keys::check_known_hosts_path(&host, port, &key, path),
                None => keys::check_known_hosts(&host, port, &key),
            };
            match check {
                Ok(true) => Ok(true),
                Ok(false) if accept_new => {
                    let learned = match known_hosts {
                        Some(path) => {
                            keys::known_hosts::learn_known_hosts_path(&host, port, &key, path)
                        }
                        None => keys::known_hosts::learn_known_hosts(&host, port, &key),
                    };
                    if let Err(err) = learned {
                        warn!(error = %err, "failed to learn host key");
                        Ok(false)
                    } else {
                        warn!(host = %host, port, "learned new SSH host key");
                        Ok(true)
                    }
                }
                Ok(false) => {
                    error!(host = %host, port, "unknown SSH host key; pass --accept-new to trust it");
                    Ok(false)
                }
                Err(err) => {
                    error!(host = %host, port, error = %err, "SSH host key check failed");
                    Ok(false)
                }
            }
        }
    }

    fn auth_banner(
        &mut self,
        banner: &str,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = std::result::Result<(), Self::Error>> + Send {
        let banner = banner.to_string();
        async move {
            info!(%banner, "SSH auth banner");
            Ok(())
        }
    }

    fn disconnected(
        &mut self,
        reason: DisconnectReason<Self::Error>,
    ) -> impl std::future::Future<Output = std::result::Result<(), Self::Error>> + Send {
        async move {
            match reason {
                DisconnectReason::ReceivedDisconnect(_) => {
                    debug!(?reason, "russh disconnected");
                    Ok(())
                }
                DisconnectReason::Error(russh::Error::Disconnect) => {
                    debug!(?reason, "russh disconnected");
                    Ok(())
                }
                DisconnectReason::Error(e) => Err(e),
            }
        }
    }
}

fn is_null_known_hosts(path: &Path) -> bool {
    let value = path.to_string_lossy();
    value.eq_ignore_ascii_case("nul") || value == "/dev/null"
}

fn resolve_target(
    target: &str,
    ssh_args: &[String],
    user_arg: Option<String>,
    port_arg: Option<u16>,
    identities_arg: Vec<PathBuf>,
    config_arg: Option<PathBuf>,
    known_hosts_arg: Option<PathBuf>,
    accept_new: bool,
    insecure_ignore_host_key: bool,
    jump_args: Vec<String>,
) -> Result<Target> {
    let (user_from_target, host_part, port_from_target) = parse_target(target);
    let default_known_hosts = if insecure_ignore_host_key {
        Some(PathBuf::from("NUL"))
    } else {
        known_hosts_arg.clone()
    };
    let mut resolved = Target {
        alias: host_part.clone(),
        host: host_part,
        port: 22,
        user: whoami::username().unwrap_or_else(|_| "root".to_string()),
        identities: Vec::new(),
        known_hosts: default_known_hosts.clone(),
        accept_new: accept_new || insecure_ignore_host_key,
        insecure_ignore_host_key,
        jumps: Vec::new(),
        jump_specs: Vec::new(),
    };

    apply_config(&mut resolved, config_arg.as_deref())?;
    apply_ssh_args(&mut resolved, ssh_args)?;

    if let Some(user) = user_from_target.or(user_arg) {
        resolved.user = user;
    }
    if let Some(port) = port_from_target.or(port_arg) {
        resolved.port = port;
    }
    if !identities_arg.is_empty() {
        resolved.identities = identities_arg;
    }
    let jump_specs = if jump_args.is_empty() {
        resolved.jump_specs.clone()
    } else {
        jump_args
    };
    resolved.jumps = resolve_jumps(
        &jump_specs,
        config_arg.as_deref(),
        default_known_hosts,
        resolved.accept_new,
        resolved.insecure_ignore_host_key,
    )?;
    if resolved.host.is_empty() {
        bail!("empty SSH host");
    }
    Ok(resolved)
}

fn parse_target(target: &str) -> (Option<String>, String, Option<u16>) {
    let (user, rest) = match target.rsplit_once('@') {
        Some((user, rest)) => (Some(user.to_string()), rest),
        None => (None, target),
    };
    let (host, port) = if let Some((host, port)) = rest.rsplit_once(':') {
        if !host.contains(']') {
            (host.to_string(), port.parse().ok())
        } else {
            (rest.to_string(), None)
        }
    } else {
        (rest.to_string(), None)
    };
    (user, host, port)
}

fn apply_config(target: &mut Target, config_path: Option<&Path>) -> Result<()> {
    let path = config_path
        .map(Path::to_path_buf)
        .or_else(|| dirs::home_dir().map(|home| home.join(".ssh").join("config")));
    let Some(path) = path else {
        return Ok(());
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let config_home = config_home_from_path(&path);

    let mut active = true;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = split_directive(line);
        if key.eq_ignore_ascii_case("Host") {
            active = value
                .split_whitespace()
                .any(|pattern| host_pattern_matches(pattern, &target.alias));
            continue;
        }
        if !active {
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "hostname" => target.host = expand_ssh_tokens(value, target),
            "user" => target.user = expand_ssh_tokens(value, target),
            "port" => {
                if let Ok(port) = value.parse() {
                    target.port = port;
                }
            }
            "identityfile" => target.identities.push(expand_path_from_config(
                &expand_ssh_tokens(value, target),
                config_home.as_deref(),
            )),
            "userknownhostsfile" => {
                target.known_hosts = Some(expand_path_from_config(
                    &expand_ssh_tokens(value, target),
                    config_home.as_deref(),
                ))
            }
            "stricthostkeychecking" if value.eq_ignore_ascii_case("accept-new") => {
                target.accept_new = true;
            }
            "stricthostkeychecking" if value.eq_ignore_ascii_case("no") => {
                target.accept_new = true;
                target.known_hosts = Some(PathBuf::from("NUL"));
                target.insecure_ignore_host_key = true;
            }
            "proxyjump" => {
                if !value.eq_ignore_ascii_case("none") {
                    target.jump_specs = value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToOwned::to_owned)
                        .collect();
                }
            }
            "proxycommand" => {
                warn!(directive = %key, "OpenSSH ProxyCommand is not implemented in russh mode");
            }
            _ => {}
        }
    }
    Ok(())
}

fn config_home_from_path(path: &Path) -> Option<PathBuf> {
    let ssh_dir = path.parent()?;
    let name = ssh_dir.file_name()?.to_string_lossy();
    if !name.eq_ignore_ascii_case(".ssh") {
        return None;
    }
    ssh_dir.parent().map(Path::to_path_buf)
}

fn split_directive(line: &str) -> (&str, &str) {
    if let Some((key, value)) = line.split_once(char::is_whitespace) {
        (key.trim(), value.trim().trim_matches('"'))
    } else if let Some((key, value)) = line.split_once('=') {
        (key.trim(), value.trim().trim_matches('"'))
    } else {
        (line, "")
    }
}

fn host_pattern_matches(pattern: &str, host: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern.starts_with('!') {
        return false;
    }
    wildcard_match(pattern, host)
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    if !pattern.contains('*') {
        return pattern.eq_ignore_ascii_case(value);
    }
    let mut rest = value;
    let mut first = true;
    for part in pattern.split('*') {
        if part.is_empty() {
            continue;
        }
        if first && !pattern.starts_with('*') {
            if let Some(next) = rest.strip_prefix(part) {
                rest = next;
            } else {
                return false;
            }
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
        first = false;
    }
    pattern.ends_with('*') || rest.is_empty()
}

fn expand_ssh_tokens(value: &str, target: &Target) -> String {
    value
        .replace("%h", &target.host)
        .replace("%n", &target.alias)
        .replace("%p", &target.port.to_string())
        .replace("%r", &target.user)
}

fn expand_path(value: &str) -> PathBuf {
    expand_path_from_config(value, None)
}

fn expand_path_from_config(value: &str, config_home: Option<&Path>) -> PathBuf {
    if let Some(stripped) = value.strip_prefix("~/") {
        if let Some(home) = config_home {
            return home.join(stripped);
        }
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(value)
}

fn apply_ssh_args(target: &mut Target, args: &[String]) -> Result<()> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-l" | "-i" | "-p" | "-F" | "-o" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow!("missing value after {arg}"))?;
                apply_ssh_option(target, arg, value)?;
            }
            _ if arg.starts_with("-l") && arg.len() > 2 => target.user = arg[2..].to_string(),
            _ if arg.starts_with("-i") && arg.len() > 2 => {
                target.identities.push(expand_path(&arg[2..]))
            }
            _ if arg.starts_with("-p") && arg.len() > 2 => target.port = arg[2..].parse()?,
            _ if arg.starts_with("-o") && arg.len() > 2 => apply_o_option(target, &arg[2..])?,
            _ => warn!(arg = %arg, "unsupported --ssh-arg ignored by russh mode"),
        }
    }
    Ok(())
}

fn apply_ssh_option(target: &mut Target, arg: &str, value: &str) -> Result<()> {
    match arg {
        "-l" => target.user = value.to_string(),
        "-i" => target.identities.push(expand_path(value)),
        "-p" => target.port = value.parse()?,
        "-F" => apply_config(target, Some(Path::new(value)))?,
        "-o" => apply_o_option(target, value)?,
        _ => {}
    }
    Ok(())
}

fn apply_o_option(target: &mut Target, value: &str) -> Result<()> {
    let (key, val) = value
        .split_once('=')
        .ok_or_else(|| anyhow!("unsupported -o format: {value}"))?;
    match key.to_ascii_lowercase().as_str() {
        "hostname" => target.host = val.to_string(),
        "user" => target.user = val.to_string(),
        "port" => target.port = val.parse()?,
        "identityfile" => target.identities.push(expand_path(val)),
        "userknownhostsfile" => target.known_hosts = Some(expand_path(val)),
        "stricthostkeychecking" if val.eq_ignore_ascii_case("accept-new") => {
            target.accept_new = true
        }
        "stricthostkeychecking" if val.eq_ignore_ascii_case("no") => {
            target.accept_new = true;
            target.known_hosts = Some(PathBuf::from("NUL"));
            target.insecure_ignore_host_key = true;
        }
        "proxyjump" if !val.eq_ignore_ascii_case("none") => {
            target.jump_specs = val
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
        _ => warn!(option = %key, "unsupported -o option ignored by russh mode"),
    }
    Ok(())
}

fn resolve_jumps(
    specs: &[String],
    config_path: Option<&Path>,
    known_hosts: Option<PathBuf>,
    accept_new: bool,
    insecure_ignore_host_key: bool,
) -> Result<Vec<Target>> {
    let mut jumps = Vec::new();
    for spec in specs {
        if spec.eq_ignore_ascii_case("none") {
            continue;
        }
        for hop in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (user_from_target, host_part, port_from_target) = parse_target(hop);
            let mut target = Target {
                alias: host_part.clone(),
                host: host_part,
                port: 22,
                user: whoami::username().unwrap_or_else(|_| "root".to_string()),
                identities: Vec::new(),
                known_hosts: known_hosts.clone(),
                accept_new,
                insecure_ignore_host_key,
                jumps: Vec::new(),
                jump_specs: Vec::new(),
            };
            apply_config(&mut target, config_path)?;
            if let Some(user) = user_from_target {
                target.user = user;
            }
            if let Some(port) = port_from_target {
                target.port = port;
            }
            target.jumps.clear();
            target.jump_specs.clear();
            jumps.push(target);
        }
    }
    Ok(jumps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proxy_jump_chain() {
        let target = resolve_target(
            "user@example.com:2222",
            &[],
            None,
            None,
            Vec::new(),
            None,
            None,
            false,
            false,
            vec!["alice@jump1:2200,jump2".to_string()],
        )
        .unwrap();

        assert_eq!(target.user, "user");
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, 2222);
        assert_eq!(target.jumps.len(), 2);
        assert_eq!(target.jumps[0].user, "alice");
        assert_eq!(target.jumps[0].host, "jump1");
        assert_eq!(target.jumps[0].port, 2200);
        assert_eq!(target.jumps[1].host, "jump2");
    }

    #[test]
    fn expands_config_relative_home_paths() {
        let home = Path::new("C:/Users/whl");
        assert_eq!(
            expand_path_from_config("~/.ssh/id_ed25519", Some(home)),
            PathBuf::from("C:/Users/whl")
                .join(".ssh")
                .join("id_ed25519")
        );
        assert_eq!(
            config_home_from_path(Path::new("C:/Users/whl/.ssh/config")),
            Some(PathBuf::from("C:/Users/whl"))
        );
    }
}
