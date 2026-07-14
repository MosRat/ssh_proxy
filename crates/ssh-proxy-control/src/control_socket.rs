#[cfg(unix)]
use std::path::PathBuf;
use std::{
    fmt,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::{Context as AnyhowContext, Result, bail};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time;

#[cfg(windows)]
mod windows_pipe;

pub const MAX_CONTROL_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_CONTROL_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const CONTROL_IO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEndpoint {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    NamedPipe(String),
}

impl ControlEndpoint {
    pub fn parse(value: &str) -> Result<Self> {
        if let Some(rest) = value.strip_prefix("tcp://") {
            return Ok(Self::Tcp(rest.parse().with_context(|| {
                format!("invalid TCP control endpoint {value:?}")
            })?));
        }
        #[cfg(unix)]
        if let Some(rest) = value.strip_prefix("unix://") {
            if rest.is_empty() {
                bail!("unix control endpoint path cannot be empty");
            }
            return Ok(Self::Unix(PathBuf::from(rest)));
        }
        #[cfg(windows)]
        if let Some(rest) = value.strip_prefix("npipe://") {
            if rest.is_empty() {
                bail!("named pipe endpoint cannot be empty");
            }
            return Ok(Self::NamedPipe(pipe_path(rest)));
        }
        Ok(Self::Tcp(value.parse().with_context(|| {
            format!("invalid control endpoint {value:?}")
        })?))
    }

    pub fn from_addr(addr: SocketAddr) -> Self {
        Self::Tcp(addr)
    }

    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp(_))
    }
}

impl fmt::Display for ControlEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "tcp://{addr}"),
            #[cfg(unix)]
            Self::Unix(path) => write!(f, "unix://{}", path.display()),
            #[cfg(windows)]
            Self::NamedPipe(path) => write!(f, "npipe://{}", display_pipe_name(path)),
        }
    }
}

pub enum ControlListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
    #[cfg(windows)]
    NamedPipe(String),
}

pub enum ControlStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    NamedPipeServer(tokio::net::windows::named_pipe::NamedPipeServer),
    #[cfg(windows)]
    NamedPipeClient(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl ControlListener {
    pub async fn bind(endpoint: &ControlEndpoint) -> Result<Self> {
        match endpoint {
            ControlEndpoint::Tcp(addr) => {
                Ok(Self::Tcp(TcpListener::bind(addr).await.with_context(
                    || format!("failed to bind TCP control listener {addr}"),
                )?))
            }
            #[cfg(unix)]
            ControlEndpoint::Unix(path) => {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.with_context(|| {
                        format!("failed to create control socket dir {}", parent.display())
                    })?;
                }
                if path.exists() {
                    std::fs::remove_file(path).with_context(|| {
                        format!("failed to remove stale control socket {}", path.display())
                    })?;
                }
                Ok(Self::Unix(
                    tokio::net::UnixListener::bind(path).with_context(|| {
                        format!("failed to bind unix control socket {}", path.display())
                    })?,
                ))
            }
            #[cfg(windows)]
            ControlEndpoint::NamedPipe(path) => {
                let server = create_named_pipe_server(path)?;
                drop(server);
                Ok(Self::NamedPipe(path.clone()))
            }
        }
    }

    pub async fn accept(&self) -> Result<ControlStream> {
        match self {
            Self::Tcp(listener) => {
                let (stream, _) = listener.accept().await?;
                Ok(ControlStream::Tcp(stream))
            }
            #[cfg(unix)]
            Self::Unix(listener) => {
                let (stream, _) = listener.accept().await?;
                Ok(ControlStream::Unix(stream))
            }
            #[cfg(windows)]
            Self::NamedPipe(path) => {
                let server = create_named_pipe_server(path)?;
                server
                    .connect()
                    .await
                    .with_context(|| format!("failed to accept named pipe client {path}"))?;
                Ok(ControlStream::NamedPipeServer(server))
            }
        }
    }
}

pub async fn request(endpoint: &ControlEndpoint, command: &str) -> Result<String> {
    if command.len() > MAX_CONTROL_REQUEST_BYTES {
        bail!("control request too large: {} bytes", command.len());
    }
    let mut stream = time::timeout(CONTROL_IO_TIMEOUT, connect(endpoint))
        .await
        .context("timed out connecting to control socket")??;
    time::timeout(
        CONTROL_IO_TIMEOUT,
        AsyncWriteExt::write_all(&mut stream, command.as_bytes()),
    )
    .await
    .context("timed out writing control request")??;
    let _ = time::timeout(CONTROL_IO_TIMEOUT, AsyncWriteExt::shutdown(&mut stream)).await;
    let mut response = String::new();
    let read = time::timeout(
        CONTROL_IO_TIMEOUT,
        AsyncReadExt::take(&mut stream, (MAX_CONTROL_RESPONSE_BYTES + 1) as u64)
            .read_to_string(&mut response),
    )
    .await
    .context("timed out reading control response")??;
    if read > MAX_CONTROL_RESPONSE_BYTES || response.len() > MAX_CONTROL_RESPONSE_BYTES {
        bail!("control response too large");
    }
    Ok(response)
}

async fn connect(endpoint: &ControlEndpoint) -> Result<ControlStream> {
    match endpoint {
        ControlEndpoint::Tcp(addr) => Ok(ControlStream::Tcp(
            TcpStream::connect(addr)
                .await
                .with_context(|| format!("failed to connect control socket {addr}"))?,
        )),
        #[cfg(unix)]
        ControlEndpoint::Unix(path) => Ok(ControlStream::Unix(
            tokio::net::UnixStream::connect(path)
                .await
                .with_context(|| {
                    format!("failed to connect unix control socket {}", path.display())
                })?,
        )),
        #[cfg(windows)]
        ControlEndpoint::NamedPipe(path) => Ok(ControlStream::NamedPipeClient(
            tokio::net::windows::named_pipe::ClientOptions::new()
                .open(path)
                .with_context(|| format!("failed to connect named pipe {path}"))?,
        )),
    }
}

impl AsyncRead for ControlStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(windows)]
            Self::NamedPipeServer(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(windows)]
            Self::NamedPipeClient(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ControlStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(windows)]
            Self::NamedPipeServer(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(windows)]
            Self::NamedPipeClient(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(windows)]
            Self::NamedPipeServer(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(windows)]
            Self::NamedPipeClient(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(windows)]
            Self::NamedPipeServer(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(windows)]
            Self::NamedPipeClient(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

#[cfg(windows)]
fn create_named_pipe_server(
    path: &str,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    windows_pipe::create_server(path)
}

#[cfg(windows)]
fn pipe_path(name: &str) -> String {
    if name.starts_with(r"\\.\pipe\") {
        name.to_string()
    } else {
        format!(r"\\.\pipe\{}", name.replace('/', r"\"))
    }
}

#[cfg(windows)]
fn display_pipe_name(path: &str) -> String {
    path.strip_prefix(r"\\.\pipe\").unwrap_or(path).to_string()
}

pub fn default_endpoint_string() -> String {
    #[cfg(windows)]
    {
        let user = endpoint_user_component();
        format!("npipe://ssh_proxy/{user}/control")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(home) = ssh_proxy_config::paths::app_home() {
            if std::env::var_os("SSH_PROXY_HOME").is_some() {
                return format!("unix://{}/control.sock", home.display());
            }
        }
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            return format!("unix://{runtime_dir}/ssh_proxy.sock");
        }
        if let Some(home) = dirs::home_dir() {
            return format!("unix://{}/.ssh_proxy/control.sock", home.display());
        }
        let user = endpoint_user_component();
        format!("tcp://127.0.0.1:{}", user_port(1081, &user))
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = ssh_proxy_config::paths::app_home() {
            if std::env::var_os("SSH_PROXY_HOME").is_some() {
                return format!("unix://{}/control.sock", home.display());
            }
        }
        if let Some(home) = dirs::home_dir() {
            return format!("unix://{}/.ssh_proxy/control.sock", home.display());
        }
        let user = endpoint_user_component();
        format!("unix:///tmp/ssh_proxy-{user}.sock")
    }
}

pub fn default_user_tcp_addr(base: u16) -> SocketAddr {
    let user = endpoint_user_component();
    SocketAddr::from(([127, 0, 0, 1], user_port(base, &user)))
}

fn endpoint_user_component() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "default".to_string())
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn user_port(base: u16, user: &str) -> u16 {
    let hash = user.bytes().fold(0_u16, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u16)
    });
    base + (hash % 1000)
}
