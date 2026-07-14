use anyhow::{Context, Result, anyhow, bail};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ProxyEndpoint {
    pub(super) host: String,
    pub(super) port: u16,
}

pub(super) fn parse_proxy_endpoint(value: &str, default_port: u16) -> Result<ProxyEndpoint> {
    let authority_end = value.find(['/', '?', '#']).unwrap_or(value.len());
    let authority = &value[..authority_end];
    let authority = authority
        .rsplit_once('@')
        .map(|(_, endpoint)| endpoint)
        .unwrap_or(authority);
    if authority.is_empty() {
        bail!("proxy endpoint is missing a host in {value:?}");
    }

    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| anyhow!("invalid bracketed IPv6 proxy endpoint {value:?}"))?;
        let host = &rest[..end];
        let tail = &rest[end + 1..];
        let port = if let Some(port) = tail.strip_prefix(':') {
            port.parse()
                .with_context(|| format!("invalid proxy port in {value:?}"))?
        } else if tail.is_empty() {
            default_port
        } else {
            bail!("invalid IPv6 proxy endpoint suffix in {value:?}");
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => {
                let port = port
                    .parse()
                    .with_context(|| format!("invalid proxy port in {value:?}"))?;
                (host, port)
            }
            _ => (authority, default_port),
        }
    };

    if host.is_empty() {
        bail!("proxy endpoint is missing a host in {value:?}");
    }
    Ok(ProxyEndpoint {
        host: host.to_string(),
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(host: &str, port: u16) -> ProxyEndpoint {
        ProxyEndpoint {
            host: host.to_string(),
            port,
        }
    }

    #[test]
    fn parses_proxy_endpoint_authority_forms() {
        let cases = [
            ("127.0.0.1:10808/", 8080, endpoint("127.0.0.1", 10808)),
            (
                "user:pass@proxy.local:3128/path?x=1",
                8080,
                endpoint("proxy.local", 3128),
            ),
            ("[::1]:1080/", 1080, endpoint("::1", 1080)),
            ("proxy.local/", 8080, endpoint("proxy.local", 8080)),
            (
                "proxy.local?source=env",
                8080,
                endpoint("proxy.local", 8080),
            ),
            ("proxy.local#fragment", 8080, endpoint("proxy.local", 8080)),
        ];

        for (value, default_port, expected) in cases {
            assert_eq!(parse_proxy_endpoint(value, default_port).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_invalid_proxy_endpoint_authorities() {
        for value in ["", ":1080", "[::1", "[::1]extra", "proxy.local:abc"] {
            assert!(
                parse_proxy_endpoint(value, 8080).is_err(),
                "{value:?} should be rejected"
            );
        }
    }
}
