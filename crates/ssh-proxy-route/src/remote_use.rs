use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteUseConnectMode {
    Auto,
    Direct,
    ReverseLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteUsePlan {
    Direct(SocketAddr),
    ReverseLink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUseDecision {
    pub plan: RemoteUsePlan,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteUseInput {
    pub connect_mode: RemoteUseConnectMode,
    pub local_peer: Option<SocketAddr>,
    pub daemon_transport_listen: Option<SocketAddr>,
}

pub fn decide_remote_use(input: &RemoteUseInput) -> Result<RemoteUseDecision, String> {
    match input.connect_mode {
        RemoteUseConnectMode::ReverseLink => {
            return Ok(RemoteUseDecision {
                plan: RemoteUsePlan::ReverseLink,
                fallback_reason: Some("--connect-mode reverse-link requested".to_string()),
            });
        }
        RemoteUseConnectMode::Direct => {
            return resolve_remote_use_local_peer(input).map(|addr| RemoteUseDecision {
                plan: RemoteUsePlan::Direct(addr),
                fallback_reason: None,
            });
        }
        RemoteUseConnectMode::Auto => {}
    }

    match resolve_remote_use_local_peer(input) {
        Ok(addr) => Ok(RemoteUseDecision {
            plan: RemoteUsePlan::Direct(addr),
            fallback_reason: None,
        }),
        Err(err) => Ok(RemoteUseDecision {
            plan: RemoteUsePlan::ReverseLink,
            fallback_reason: Some(err),
        }),
    }
}

pub fn resolve_remote_use_local_peer(input: &RemoteUseInput) -> Result<SocketAddr, String> {
    if let Some(addr) = input.local_peer {
        return Ok(addr);
    }
    let Some(addr) = input.daemon_transport_listen else {
        return Err(
            "--direction remote-uses-local needs --local-peer or [daemon].transport_listen; run `ssh_proxy daemon install --scope system --elevate` first"
                .to_string(),
        );
    };
    if addr.ip().is_loopback() {
        return Err(format!(
            "local daemon transport {addr} is loopback-only; pass --local-peer <reachable-ip:port>, or use a public/TLS/QUIC relay route when this machine is behind NAT"
        ));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(connect_mode: RemoteUseConnectMode) -> RemoteUseInput {
        RemoteUseInput {
            connect_mode,
            local_peer: None,
            daemon_transport_listen: None,
        }
    }

    #[test]
    fn direct_mode_requires_reachable_local_peer() {
        let err = decide_remote_use(&RemoteUseInput {
            daemon_transport_listen: Some("127.0.0.1:19080".parse().unwrap()),
            ..input(RemoteUseConnectMode::Direct)
        })
        .unwrap_err();

        assert!(err.contains("loopback-only"));
    }

    #[test]
    fn auto_falls_back_to_reverse_link_when_peer_is_loopback_only() {
        let decision = decide_remote_use(&RemoteUseInput {
            daemon_transport_listen: Some("127.0.0.1:19080".parse().unwrap()),
            ..input(RemoteUseConnectMode::Auto)
        })
        .expect("remote use decision");

        assert!(matches!(decision.plan, RemoteUsePlan::ReverseLink));
        assert!(
            decision
                .fallback_reason
                .as_deref()
                .expect("fallback reason")
                .contains("loopback-only")
        );
    }

    #[test]
    fn auto_uses_reachable_local_peer() {
        let decision = decide_remote_use(&RemoteUseInput {
            daemon_transport_listen: Some("192.0.2.8:19080".parse().unwrap()),
            ..input(RemoteUseConnectMode::Auto)
        })
        .expect("remote use decision");

        assert!(matches!(decision.plan, RemoteUsePlan::Direct(_)));
        assert_eq!(decision.fallback_reason, None);
    }

    #[test]
    fn explicit_reverse_link_records_the_requested_fallback_reason() {
        let decision = decide_remote_use(&input(RemoteUseConnectMode::ReverseLink))
            .expect("remote use decision");

        assert!(matches!(decision.plan, RemoteUsePlan::ReverseLink));
        assert_eq!(
            decision.fallback_reason.as_deref(),
            Some("--connect-mode reverse-link requested")
        );
    }

    #[test]
    fn local_peer_argument_wins_over_daemon_default() {
        let decision = decide_remote_use(&RemoteUseInput {
            local_peer: Some("192.0.2.9:19080".parse().unwrap()),
            daemon_transport_listen: Some("127.0.0.1:19080".parse().unwrap()),
            ..input(RemoteUseConnectMode::Direct)
        })
        .expect("remote use decision");

        assert!(matches!(
            decision.plan,
            RemoteUsePlan::Direct(addr) if addr.to_string() == "192.0.2.9:19080"
        ));
    }
}
