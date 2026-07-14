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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpResponseBodyMode {
    ContentLength(u64),
    Chunked,
    CloseDelimited,
    NoBody,
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
            if http_header_end(&bytes).is_some() {
                break;
            }
            if bytes.len() > 64 * 1024 {
                bail!("HTTP proxy request headers too large");
            }
        }
        let header_end =
            http_header_end(&bytes).ok_or_else(|| anyhow!("missing HTTP header terminator"))?;
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
        let header_lines = lines
            .take_while(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let connection_tokens = connection_header_tokens(&header_lines);
        for line in header_lines {
            if should_skip_forward_header(line, &connection_tokens) {
                continue;
            }
            rewritten.extend_from_slice(line.as_bytes());
            rewritten.extend_from_slice(b"\r\n");
        }
        rewritten.extend_from_slice(b"connection: close\r\n");
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

pub fn rewrite_response_head_for_proxy_close(
    head: &[u8],
) -> Result<(Vec<u8>, HttpResponseBodyMode)> {
    let headers = std::str::from_utf8(head).context("HTTP proxy response headers are not utf-8")?;
    let mut lines = headers.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| anyhow!("missing HTTP response status line"))?;
    let status = parse_response_status(status_line)?;
    let header_lines = lines
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let connection_tokens = connection_header_tokens(&header_lines);
    let mut content_length = None;
    let mut chunked = false;

    let mut rewritten = Vec::new();
    rewritten.extend_from_slice(status_line.as_bytes());
    rewritten.extend_from_slice(b"\r\n");
    for line in header_lines {
        if let Some((name, value)) = split_header(line) {
            if name.eq_ignore_ascii_case("content-length") {
                if content_length.is_none() {
                    content_length = value.parse::<u64>().ok();
                }
            } else if name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
            {
                chunked = true;
            }
        }
        if should_skip_forward_header(line, &connection_tokens) {
            continue;
        }
        rewritten.extend_from_slice(line.as_bytes());
        rewritten.extend_from_slice(b"\r\n");
    }
    rewritten.extend_from_slice(b"connection: close\r\n\r\n");

    let body_mode = if response_status_has_no_body(status) {
        HttpResponseBodyMode::NoBody
    } else if chunked {
        HttpResponseBodyMode::Chunked
    } else if let Some(length) = content_length {
        HttpResponseBodyMode::ContentLength(length)
    } else {
        HttpResponseBodyMode::CloseDelimited
    };

    Ok((rewritten, body_mode))
}

pub fn http_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn parse_response_status(status_line: &str) -> Result<u16> {
    let mut parts = status_line.split_whitespace();
    let version = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP response version"))?;
    if !version.starts_with("HTTP/") {
        bail!("invalid HTTP response status line");
    }
    let status = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP response status"))?
        .parse::<u16>()
        .context("invalid HTTP response status")?;
    Ok(status)
}

fn response_status_has_no_body(status: u16) -> bool {
    (100..200).contains(&status) || matches!(status, 204 | 304)
}

fn connection_header_tokens(headers: &[&str]) -> Vec<String> {
    headers
        .iter()
        .filter_map(|line| split_header(line))
        .filter(|(name, _)| name.eq_ignore_ascii_case("connection"))
        .flat_map(|(_, value)| value.split(','))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn should_skip_forward_header(line: &str, connection_tokens: &[String]) -> bool {
    let Some((name, _)) = split_header(line) else {
        return false;
    };
    is_static_hop_by_hop_header(name)
        || connection_tokens
            .iter()
            .any(|token| name.eq_ignore_ascii_case(token))
}

fn split_header(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once(':')?;
    Some((name.trim(), value.trim()))
}

fn is_static_hop_by_hop_header(name: &str) -> bool {
    [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "upgrade",
    ]
    .iter()
    .any(|header| name.eq_ignore_ascii_case(header))
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
