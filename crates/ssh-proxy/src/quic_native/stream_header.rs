#![allow(dead_code)]

use anyhow::{Context, Result, anyhow, bail};

pub const STREAM_HEADER_MAGIC: &[u8; 4] = b"QNT1";
pub const STREAM_HEADER_VERSION: u16 = 1;
pub const MAX_STREAM_HEADER: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    TcpConnect,
    FixedTcp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamTarget {
    pub kind: TargetKind,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamHeader {
    pub route_id: String,
    pub stream_id: u64,
    pub target: StreamTarget,
    pub egress_proxy: Option<String>,
    pub flags: u32,
}

impl StreamHeader {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.encoded_capacity_hint());
        self.encode_into(&mut out)?;
        Ok(out)
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<()> {
        out.clear();
        out.extend_from_slice(STREAM_HEADER_MAGIC);
        out.extend_from_slice(&STREAM_HEADER_VERSION.to_be_bytes());
        out.extend_from_slice(&0_u16.to_be_bytes());
        let body_start = out.len();
        out.extend_from_slice(&self.flags.to_be_bytes());
        out.extend_from_slice(&self.stream_id.to_be_bytes());
        out.push(self.target.kind.to_wire());
        write_string(out, &self.route_id)?;
        write_string(out, &self.target.host)?;
        out.extend_from_slice(&self.target.port.to_be_bytes());
        match &self.egress_proxy {
            Some(proxy) => {
                out.push(1);
                write_string(out, proxy)?;
            }
            None => out.push(0),
        }
        let body_len = out.len() - body_start;
        if body_len > MAX_STREAM_HEADER {
            bail!("QUIC-native stream header too large: {body_len}");
        }
        let body_len = (body_len as u16).to_be_bytes();
        out[6] = body_len[0];
        out[7] = body_len[1];
        Ok(())
    }

    pub fn encoded_capacity_hint(&self) -> usize {
        let egress_len = self
            .egress_proxy
            .as_ref()
            .map_or(0, |proxy| 2 + proxy.len());
        8 + 4 + 8 + 1 + 2 + self.route_id.len() + 2 + self.target.host.len() + 2 + 1 + egress_len
    }

    pub fn decode(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < 8 {
            bail!("QUIC-native stream header is truncated");
        }
        if &bytes[..4] != STREAM_HEADER_MAGIC {
            bail!("invalid QUIC-native stream header magic");
        }
        let version = u16::from_be_bytes(bytes[4..6].try_into()?);
        if version != STREAM_HEADER_VERSION {
            bail!(
                "unsupported QUIC-native stream header version {version}; expected {STREAM_HEADER_VERSION}"
            );
        }
        let body_len = u16::from_be_bytes(bytes[6..8].try_into()?) as usize;
        if body_len > MAX_STREAM_HEADER {
            bail!("QUIC-native stream header too large: {body_len}");
        }
        let end = 8 + body_len;
        let body = bytes
            .get(8..end)
            .ok_or_else(|| anyhow!("QUIC-native stream header payload is truncated"))?;
        let mut cursor = Cursor::new(body);
        let flags = cursor.read_u32()?;
        let stream_id = cursor.read_u64()?;
        let kind = TargetKind::from_wire(cursor.read_u8()?)?;
        let route_id = cursor.read_string()?;
        let host = cursor.read_string()?;
        let port = cursor.read_u16()?;
        let egress_proxy = match cursor.read_u8()? {
            0 => None,
            1 => Some(cursor.read_string()?),
            other => bail!("invalid QUIC-native egress proxy marker {other}"),
        };
        cursor.ensure_empty()?;
        Ok((
            Self {
                route_id,
                stream_id,
                target: StreamTarget { kind, host, port },
                egress_proxy,
                flags,
            },
            end,
        ))
    }
}

impl TargetKind {
    fn to_wire(self) -> u8 {
        match self {
            Self::TcpConnect => 1,
            Self::FixedTcp => 2,
        }
    }

    fn from_wire(value: u8) -> Result<Self> {
        match value {
            1 => Ok(Self::TcpConnect),
            2 => Ok(Self::FixedTcp),
            other => bail!("unknown QUIC-native target kind {other}"),
        }
    }
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() > u16::MAX as usize {
        bail!("QUIC-native header string too long");
    }
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8> {
        let value = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| anyhow!("QUIC-native stream header payload truncated"))?;
        self.pos += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_be_bytes(bytes.try_into()?))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_be_bytes(bytes.try_into()?))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_be_bytes(bytes.try_into()?))
    }

    fn read_string(&mut self) -> Result<String> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_bytes(len)?;
        String::from_utf8(bytes.to_vec()).context("invalid utf-8 in QUIC-native header")
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("QUIC-native stream header length overflow"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| anyhow!("QUIC-native stream header payload truncated"))?;
        self.pos = end;
        Ok(bytes)
    }

    fn ensure_empty(&self) -> Result<()> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            bail!(
                "QUIC-native stream header has {} trailing bytes",
                self.bytes.len() - self.pos
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_header_round_trips() {
        let header = StreamHeader {
            route_id: "local-via-remote:peer-a:18080".to_string(),
            stream_id: 42,
            target: StreamTarget {
                kind: TargetKind::TcpConnect,
                host: "example.com".to_string(),
                port: 443,
            },
            egress_proxy: Some("socks5h://127.0.0.1:18080".to_string()),
            flags: 0x10,
        };

        let encoded = header.encode().unwrap();
        let (decoded, consumed) = StreamHeader::decode(&encoded).unwrap();

        assert_eq!(decoded, header);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn stream_header_encode_into_reuses_buffer() {
        let header = StreamHeader {
            route_id: "route".to_string(),
            stream_id: 1,
            target: StreamTarget {
                kind: TargetKind::FixedTcp,
                host: "127.0.0.1".to_string(),
                port: 80,
            },
            egress_proxy: None,
            flags: 0,
        };
        let mut encoded = Vec::with_capacity(header.encoded_capacity_hint());

        header.encode_into(&mut encoded).unwrap();
        let first_capacity = encoded.capacity();
        header.encode_into(&mut encoded).unwrap();

        assert_eq!(encoded.capacity(), first_capacity);
        let (decoded, consumed) = StreamHeader::decode(&encoded).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(consumed, encoded.len());
    }

    #[test]
    fn stream_header_rejects_wrong_magic() {
        let mut encoded = StreamHeader {
            route_id: "route".to_string(),
            stream_id: 1,
            target: StreamTarget {
                kind: TargetKind::FixedTcp,
                host: "127.0.0.1".to_string(),
                port: 80,
            },
            egress_proxy: None,
            flags: 0,
        }
        .encode()
        .unwrap();
        encoded[0] = b'X';

        let err = StreamHeader::decode(&encoded).unwrap_err().to_string();

        assert!(err.contains("magic"), "{err}");
    }
}
