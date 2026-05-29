use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
    time,
};

mod proxy_endpoint;

use proxy_endpoint::parse_proxy_endpoint;

const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);
const PROXY_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn connect_tcp(
    host: &str,
    port: u16,
    egress_proxy: Option<&str>,
) -> Result<TcpStream> {
    if let Some(proxy) = egress_proxy {
        return connect_via_upstream_proxy(proxy, host, port).await;
    }

    connect_direct_tcp(host, port).await
}

async fn connect_direct_tcp(host: &str, port: u16) -> Result<TcpStream> {
    let mut addrs: Vec<SocketAddr> = lookup_host((host, port))
        .await
        .with_context(|| format!("failed to resolve {host}:{port}"))?
        .collect();
    addrs.sort_by_key(|addr| if addr.is_ipv4() { 0 } else { 1 });
    addrs.dedup();

    if addrs.is_empty() {
        bail!("no resolved addresses for {host}:{port}");
    }

    let mut errors = Vec::new();
    for addr in addrs {
        match time::timeout(CONNECT_ATTEMPT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(err)) => errors.push(format!("{addr}: {err}")),
            Err(_) => errors.push(format!(
                "{addr}: connect timed out after {}s",
                CONNECT_ATTEMPT_TIMEOUT.as_secs()
            )),
        }
    }

    Err(anyhow!(
        "failed to connect {host}:{port}; attempts: {}",
        errors.join("; ")
    ))
}

async fn connect_via_upstream_proxy(proxy: &str, host: &str, port: u16) -> Result<TcpStream> {
    if let Some(addr) = proxy.strip_prefix("http://") {
        return time::timeout(
            PROXY_HANDSHAKE_TIMEOUT,
            connect_via_http_proxy(addr, host, port),
        )
        .await
        .with_context(|| {
            format!(
                "upstream HTTP proxy handshake timed out after {}s",
                PROXY_HANDSHAKE_TIMEOUT.as_secs()
            )
        })?;
    }

    if let Some(addr) = proxy
        .strip_prefix("socks5h://")
        .or_else(|| proxy.strip_prefix("socks5://"))
    {
        return time::timeout(
            PROXY_HANDSHAKE_TIMEOUT,
            connect_via_socks5_proxy(addr, host, port),
        )
        .await
        .with_context(|| {
            format!(
                "upstream SOCKS5 proxy handshake timed out after {}s",
                PROXY_HANDSHAKE_TIMEOUT.as_secs()
            )
        })?;
    }

    bail!("unsupported egress proxy scheme {proxy:?}; use http:// or socks5h://")
}

async fn connect_via_http_proxy(addr: &str, host: &str, port: u16) -> Result<TcpStream> {
    let endpoint = parse_proxy_endpoint(addr, 8080)?;
    let mut stream = connect_direct_tcp(&endpoint.host, endpoint.port).await?;
    let request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = Vec::with_capacity(1024);
    let mut buf = [0_u8; 1];
    while response.len() < 8192 {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        response.push(buf[0]);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&response);
    if text.starts_with("HTTP/1.1 2") || text.starts_with("HTTP/1.0 2") {
        return Ok(stream);
    }
    bail!(
        "upstream HTTP proxy CONNECT failed: {}",
        text.lines().next().unwrap_or("")
    )
}

async fn connect_via_socks5_proxy(addr: &str, host: &str, port: u16) -> Result<TcpStream> {
    let endpoint = parse_proxy_endpoint(addr, 1080)?;
    let mut stream = connect_direct_tcp(&endpoint.host, endpoint.port).await?;
    stream.write_all(&[5, 1, 0]).await?;
    let mut method = [0_u8; 2];
    stream.read_exact(&mut method).await?;
    if method != [5, 0] {
        bail!("upstream SOCKS5 proxy rejected no-auth method");
    }

    let host_bytes = host.as_bytes();
    if host_bytes.len() > u8::MAX as usize {
        bail!("target host name is too long for SOCKS5");
    }
    let mut request = Vec::with_capacity(7 + host_bytes.len());
    request.extend_from_slice(&[5, 1, 0, 3, host_bytes.len() as u8]);
    request.extend_from_slice(host_bytes);
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut head = [0_u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != 5 || head[1] != 0 {
        bail!("upstream SOCKS5 connect failed with reply {}", head[1]);
    }
    match head[3] {
        1 => {
            let mut skip = [0_u8; 6];
            stream.read_exact(&mut skip).await?;
        }
        3 => {
            let len = stream.read_u8().await? as usize;
            let mut skip = vec![0_u8; len + 2];
            stream.read_exact(&mut skip).await?;
        }
        4 => {
            let mut skip = [0_u8; 18];
            stream.read_exact(&mut skip).await?;
        }
        atyp => bail!("upstream SOCKS5 returned unsupported address type {atyp}"),
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn http_upstream_connects_exact_target_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut buf).await.unwrap();
                request.push(buf[0]);
            }
            let text = String::from_utf8(request).unwrap();
            assert!(text.starts_with("CONNECT example.com:8443 HTTP/1.1"));
            stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            stream.write_all(b"ready").await.unwrap();
        });

        let mut stream = connect_tcp(
            "example.com",
            8443,
            Some(&format!("http://{}", endpoint(proxy_addr))),
        )
        .await
        .unwrap();
        let mut body = [0_u8; 5];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"ready");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn socks5h_upstream_connects_exact_target_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut greeting = [0_u8; 3];
            stream.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            stream.write_all(&[5, 0]).await.unwrap();

            let mut head = [0_u8; 5];
            stream.read_exact(&mut head).await.unwrap();
            assert_eq!(&head[..4], &[5, 1, 0, 3]);
            let len = head[4] as usize;
            let mut host = vec![0_u8; len];
            stream.read_exact(&mut host).await.unwrap();
            let mut port = [0_u8; 2];
            stream.read_exact(&mut port).await.unwrap();
            assert_eq!(String::from_utf8(host).unwrap(), "example.com");
            assert_eq!(u16::from_be_bytes(port), 9443);

            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
            stream.write_all(b"ready").await.unwrap();
        });

        let mut stream = connect_tcp(
            "example.com",
            9443,
            Some(&format!("socks5h://{}", endpoint(proxy_addr))),
        )
        .await
        .unwrap();
        let mut body = [0_u8; 5];
        stream.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"ready");
        server.await.unwrap();
    }

    fn endpoint(addr: SocketAddr) -> String {
        format!("{}:{}", addr.ip(), addr.port())
    }
}
