use anyhow::{Context, Result, anyhow, bail};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub struct HttpRequest {
    pub kind: HttpRequestKind,
}

pub enum HttpRequestKind {
    Connect {
        host: String,
        port: u16,
    },
    Forward {
        host: String,
        port: u16,
        request: Vec<u8>,
    },
}

impl HttpRequest {
    pub async fn read_from(stream: &mut TcpStream) -> Result<Self> {
        let mut bytes = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let n = stream.read(&mut buf).await?;
            if n == 0 {
                bail!("HTTP proxy request closed before headers");
            }
            bytes.extend_from_slice(&buf[..n]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
            if bytes.len() > 64 * 1024 {
                bail!("HTTP proxy request headers too large");
            }
        }
        let header_end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|pos| pos + 4)
            .ok_or_else(|| anyhow!("missing HTTP header terminator"))?;
        let headers = std::str::from_utf8(&bytes[..header_end])
            .context("HTTP proxy headers are not utf-8")?;
        let mut lines = headers.split("\r\n");
        let request_line = lines
            .next()
            .ok_or_else(|| anyhow!("missing HTTP request line"))?;
        let mut parts = request_line.split_whitespace();
        let method = parts.next().ok_or_else(|| anyhow!("missing HTTP method"))?;
        let target = parts.next().ok_or_else(|| anyhow!("missing HTTP target"))?;
        let version = parts
            .next()
            .ok_or_else(|| anyhow!("missing HTTP version"))?;
        if parts.next().is_some() || !version.starts_with("HTTP/") {
            bail!("invalid HTTP request line");
        }
        if method.eq_ignore_ascii_case("CONNECT") {
            let (host, port) = parse_host_port(target, 443)?;
            return Ok(Self {
                kind: HttpRequestKind::Connect { host, port },
            });
        }

        let absolute = parse_absolute_http_target(target)
            .ok_or_else(|| anyhow!("HTTP proxy only supports CONNECT or absolute-form URLs"))?;
        let mut rewritten = Vec::new();
        rewritten.extend_from_slice(method.as_bytes());
        rewritten.push(b' ');
        rewritten.extend_from_slice(absolute.path.as_bytes());
        rewritten.push(b' ');
        rewritten.extend_from_slice(version.as_bytes());
        rewritten.extend_from_slice(b"\r\n");
        for line in lines {
            if line.is_empty() {
                break;
            }
            rewritten.extend_from_slice(line.as_bytes());
            rewritten.extend_from_slice(b"\r\n");
        }
        rewritten.extend_from_slice(b"\r\n");
        rewritten.extend_from_slice(&bytes[header_end..]);
        Ok(Self {
            kind: HttpRequestKind::Forward {
                host: absolute.host,
                port: absolute.port,
                request: rewritten,
            },
        })
    }
}

struct AbsoluteTarget {
    host: String,
    port: u16,
    path: String,
}

fn parse_absolute_http_target(target: &str) -> Option<AbsoluteTarget> {
    let rest = target.strip_prefix("http://")?;
    let (authority, path) = match rest.find(['/', '?']) {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };
    let path = if path.starts_with('?') {
        format!("/{path}")
    } else {
        path.to_string()
    };
    let (host, port) = parse_host_port(authority, 80).ok()?;
    Some(AbsoluteTarget { host, port, path })
}

fn parse_host_port(authority: &str, default_port: u16) -> Result<(String, u16)> {
    if authority.is_empty() {
        bail!("empty HTTP authority");
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| anyhow!("invalid bracketed IPv6 authority"))?;
        let host = rest[..end].to_string();
        let tail = &rest[end + 1..];
        let port = if let Some(port) = tail.strip_prefix(':') {
            port.parse()
                .with_context(|| format!("invalid HTTP authority port {port:?}"))?
        } else if tail.is_empty() {
            default_port
        } else {
            bail!("invalid HTTP authority suffix {tail:?}");
        };
        return Ok((host, port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            let port = port
                .parse()
                .with_context(|| format!("invalid HTTP authority port {port:?}"))?;
            Ok((host.to_string(), port))
        }
        _ => Ok((authority.to_string(), default_port)),
    }
}

pub async fn write_http_error(stream: &mut TcpStream, status: u16, reason: &str) -> Result<()> {
    let body = format!("{status} {reason}\n");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}
