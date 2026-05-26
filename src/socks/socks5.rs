use anyhow::{Context, Result, anyhow, bail};
use std::net::{IpAddr, SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(super) const SOCKS_VERSION: u8 = 5;

pub(super) async fn negotiate_no_auth(stream: &mut TcpStream) -> Result<()> {
    let version = stream.read_u8().await?;
    if version != SOCKS_VERSION {
        bail!("invalid SOCKS version {version}");
    }
    let methods_len = stream.read_u8().await? as usize;
    let mut methods = vec![0_u8; methods_len];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        stream.write_all(&[SOCKS_VERSION, 0xff]).await?;
        bail!("SOCKS client did not offer no-auth method");
    }
    stream.write_all(&[SOCKS_VERSION, 0]).await?;
    Ok(())
}

#[derive(Debug)]
pub(super) struct Request {
    pub(super) command: Command,
    pub(super) host: String,
    pub(super) port: u16,
}

impl Request {
    pub(super) async fn read_from(stream: &mut TcpStream) -> Result<Self> {
        let version = stream.read_u8().await?;
        if version != SOCKS_VERSION {
            bail!("invalid SOCKS request version {version}");
        }
        let command = match stream.read_u8().await? {
            1 => Command::Connect,
            2 => Command::Bind,
            3 => Command::UdpAssociate,
            other => bail!("unsupported SOCKS command {other}"),
        };
        let reserved = stream.read_u8().await?;
        if reserved != 0 {
            bail!("invalid SOCKS reserved field {reserved}");
        }
        let host = read_addr(stream).await?;
        let port = stream.read_u16().await?;
        Ok(Self {
            command,
            host,
            port,
        })
    }
}

#[derive(Debug)]
pub(super) enum Command {
    Connect,
    Bind,
    UdpAssociate,
}

#[derive(Clone, Copy)]
pub(super) enum Reply {
    Succeeded = 0,
    HostUnreachable = 4,
    CommandNotSupported = 7,
}

pub(super) async fn reply(stream: &mut TcpStream, reply: Reply, bind: SocketAddr) -> Result<()> {
    let mut out = vec![SOCKS_VERSION, reply as u8, 0];
    match bind.ip() {
        IpAddr::V4(ip) => {
            out.push(1);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(4);
            out.extend_from_slice(&ip.octets());
        }
    }
    out.extend_from_slice(&bind.port().to_be_bytes());
    stream.write_all(&out).await?;
    Ok(())
}

async fn read_addr(stream: &mut TcpStream) -> Result<String> {
    match stream.read_u8().await? {
        1 => {
            let mut octets = [0_u8; 4];
            stream.read_exact(&mut octets).await?;
            Ok(IpAddr::V4(octets.into()).to_string())
        }
        3 => {
            let len = stream.read_u8().await? as usize;
            let mut name = vec![0_u8; len];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name).context("SOCKS domain name is not utf-8")
        }
        4 => {
            let mut octets = [0_u8; 16];
            stream.read_exact(&mut octets).await?;
            Ok(IpAddr::V6(octets.into()).to_string())
        }
        other => bail!("unsupported SOCKS address type {other}"),
    }
}

#[derive(Clone)]
pub(super) struct UdpPacket {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) data: Vec<u8>,
}

pub(super) fn parse_udp_packet(bytes: &[u8]) -> Result<UdpPacket> {
    if bytes.len() < 4 || bytes[0] != 0 || bytes[1] != 0 {
        bail!("invalid SOCKS UDP packet");
    }
    if bytes[2] != 0 {
        bail!("fragmented SOCKS UDP packets are not supported");
    }
    let mut pos = 3;
    let host = match *bytes
        .get(pos)
        .ok_or_else(|| anyhow!("missing UDP address type"))?
    {
        1 => {
            pos += 1;
            let addr = bytes
                .get(pos..pos + 4)
                .ok_or_else(|| anyhow!("truncated IPv4 address"))?;
            pos += 4;
            IpAddr::V4([addr[0], addr[1], addr[2], addr[3]].into()).to_string()
        }
        3 => {
            pos += 1;
            let len = *bytes
                .get(pos)
                .ok_or_else(|| anyhow!("truncated domain length"))? as usize;
            pos += 1;
            let name = bytes
                .get(pos..pos + len)
                .ok_or_else(|| anyhow!("truncated domain name"))?;
            pos += len;
            String::from_utf8(name.to_vec()).context("UDP domain name is not utf-8")?
        }
        4 => {
            pos += 1;
            let addr = bytes
                .get(pos..pos + 16)
                .ok_or_else(|| anyhow!("truncated IPv6 address"))?;
            pos += 16;
            let mut octets = [0_u8; 16];
            octets.copy_from_slice(addr);
            IpAddr::V6(octets.into()).to_string()
        }
        other => bail!("unsupported UDP address type {other}"),
    };
    let port_bytes = bytes
        .get(pos..pos + 2)
        .ok_or_else(|| anyhow!("truncated UDP port"))?;
    let port = u16::from_be_bytes(port_bytes.try_into()?);
    pos += 2;
    Ok(UdpPacket {
        host,
        port,
        data: bytes[pos..].to_vec(),
    })
}

pub(super) fn build_udp_packet(host: &str, port: u16, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = vec![0, 0, 0];
    if let Ok(ip) = host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(ip) => {
                out.push(1);
                out.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                out.push(4);
                out.extend_from_slice(&ip.octets());
            }
        }
    } else {
        let bytes = host.as_bytes();
        if bytes.len() > u8::MAX as usize {
            bail!("UDP response hostname too long");
        }
        out.push(3);
        out.push(bytes.len() as u8);
        out.extend_from_slice(bytes);
    }
    out.extend_from_slice(&port.to_be_bytes());
    out.extend_from_slice(data);
    Ok(out)
}
