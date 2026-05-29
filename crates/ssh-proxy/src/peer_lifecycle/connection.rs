use std::net::SocketAddr;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{cli, config};

#[derive(Debug, Clone)]
pub(crate) struct TransportSelection {
    pub(crate) transport: cli::RemoteTransport,
    pub(crate) source: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConnectionDecision {
    pub(crate) selected_transport: String,
    pub(crate) source: String,
    pub(crate) reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) endpoint: Option<String>,
    #[serde(default)]
    pub(crate) requires_external_ssh: bool,
}

impl ConnectionDecision {
    pub(crate) fn from_transport(
        transport: cli::RemoteTransport,
        source: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let decision =
            ssh_proxy_route::ConnectionDecision::from_transport(transport.into(), source, reason);
        Self::from_route_decision(decision)
    }

    pub(crate) fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    fn from_route_decision(decision: ssh_proxy_route::ConnectionDecision) -> Self {
        Self {
            selected_transport: decision.selected_transport,
            source: decision.source,
            reason: decision.reason,
            endpoint: decision.endpoint,
            requires_external_ssh: decision.requires_external_ssh,
        }
    }
}

pub(crate) fn persistent_peer_ready(peer: Option<&config::PeerRecord>) -> bool {
    peer.is_some_and(|peer| {
        ssh_proxy_route::persistent_peer_ready(ssh_proxy_route::PeerReadinessInput {
            has_remote_path: peer.remote_path.is_some(),
            has_control_endpoint: peer.control_endpoint.is_some(),
            has_transport: peer.transport.is_some(),
            has_tls_transport: peer.tls_transport.is_some(),
            has_quic_transport: peer.quic_transport.is_some(),
        })
    })
}

pub(crate) fn transport_selection_policy(
    args: &cli::RouteArgs,
    profile: Option<&config::ProxyProfile>,
    defaults: &config::ProxyProfile,
    remote_quic: Option<SocketAddr>,
    remote_tls: Option<SocketAddr>,
    allow_plain_tcp: bool,
    remote_side_listens: bool,
    persistent_peer_ready: bool,
) -> Result<TransportSelection> {
    let selected = ssh_proxy_route::transport_selection_policy(
        &ssh_proxy_route::TransportSelectionPolicyInput {
            requested_transport: args.remote_transport.into(),
            profile_remote_transport: profile
                .and_then(|profile| profile.remote_transport.as_deref()),
            defaults_remote_transport: defaults.remote_transport.as_deref(),
            remote_quic,
            remote_tls,
            allow_plain_tcp,
            cli_allow_plain_tcp: args.allow_plain_tcp,
            profile_allow_plain_tcp: profile.and_then(|profile| profile.allow_plain_tcp),
            defaults_allow_plain_tcp: defaults.allow_plain_tcp,
            tcp_target_present: args.tcp_target.is_some(),
            remote_side_listens,
            persistent_peer_ready,
        },
    )
    .map_err(anyhow::Error::msg)?;
    Ok(TransportSelection {
        transport: selected.transport.into(),
        source: selected.source,
        reason: selected.reason,
    })
}

pub(crate) fn direct_endpoint_for_peer(peer: &config::PeerRecord) -> Option<SocketAddr> {
    ssh_proxy_route::direct_endpoint_for_peer(
        peer.transport,
        peer.tls_transport,
        peer.quic_transport,
    )
}

pub(crate) fn remote_transport_name(transport: cli::RemoteTransport) -> &'static str {
    ssh_proxy_route::remote_transport_name(transport.into())
}

pub(crate) fn direct_transport_policy(transport: cli::RemoteTransport) -> Value {
    ssh_proxy_route::direct_transport_policy(transport.into())
}

pub(crate) fn direct_transport_policy_reason(transport: cli::RemoteTransport) -> Value {
    ssh_proxy_route::direct_transport_policy_reason(transport.into())
}

pub(crate) fn tls_peer_auth_mode<T, U>(
    transport: cli::RemoteTransport,
    client_cert: Option<T>,
    client_key: Option<U>,
) -> Value {
    ssh_proxy_route::tls_peer_auth_mode(
        transport.into(),
        client_cert.is_some(),
        client_key.is_some(),
    )
}

pub(crate) fn ssh_mode_name(transport: cli::RemoteTransport) -> Value {
    ssh_proxy_route::ssh_mode_name(transport.into())
}

pub(crate) fn ssh_mode_reason(transport: cli::RemoteTransport) -> Value {
    ssh_proxy_route::ssh_mode_reason(transport.into())
}

pub(crate) fn ssh_data_plane_reason(
    transport: cli::RemoteTransport,
    selection_source: Option<&str>,
) -> Value {
    ssh_proxy_route::ssh_data_plane_reason(transport.into(), selection_source)
}

pub(crate) fn parse_remote_transport(value: &str) -> Result<cli::RemoteTransport> {
    ssh_proxy_route::parse_transport_mode(value)
        .map(Into::into)
        .map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn route_args() -> cli::RouteArgs {
        cli::RouteArgs {
            target: "peer".to_string(),
            direction: cli::RouteDirection::LocalUsesRemote,
            connect_mode: cli::RouteConnectMode::Auto,
            port: 18080,
            bind: "127.0.0.1".parse().unwrap(),
            tcp_target: None,
            endpoint: "tcp://127.0.0.1:1".to_string(),
            token: None,
            ssh_args: Vec::new(),
            user: None,
            ssh_port: None,
            identity: Vec::new(),
            config: None,
            known_hosts: None,
            accept_new: false,
            insecure_ignore_host_key: false,
            jump: Vec::new(),
            remote_path: None,
            remote_bin: None,
            deploy: cli::DeployMode::Auto,
            remote_os: cli::RemoteOs::Auto,
            remote_transport: cli::RemoteTransport::Auto,
            remote_tcp: None,
            remote_control: None,
            remote_quic: None,
            remote_tls: None,
            remote_ca: None,
            remote_name: "localhost".to_string(),
            remote_token: None,
            egress_proxy: None,
            reconnect_delay_secs: None,
            reconnect_max_delay_secs: None,
            connect_timeout_secs: None,
            quic_max_bidi_streams: None,
            quic_stream_receive_window: None,
            quic_receive_window: None,
            quic_keep_alive_interval_secs: None,
            quic_idle_timeout_secs: None,
            transport_pool_size: None,
            workload_hint: None,
            ssh_session_pool_size: None,
            no_reconnect: false,
            local_peer: None,
            allow_plain_tcp: false,
            id: None,
            volatile: false,
            dry_run: true,
            explain: false,
            json: false,
        }
    }

    #[test]
    fn peer_ready_requires_descriptor_and_transport() {
        assert!(!persistent_peer_ready(None));

        let mut peer = config::PeerRecord {
            remote_path: Some("~/.local/bin/ssh_proxy".to_string()),
            control_endpoint: Some("tcp://127.0.0.1:19081".to_string()),
            ..Default::default()
        };
        assert!(!persistent_peer_ready(Some(&peer)));

        peer.transport = Some("127.0.0.1:19080".parse().unwrap());
        assert!(persistent_peer_ready(Some(&peer)));
    }

    #[test]
    fn external_ssh_is_explicit_compatibility() {
        let decision = ConnectionDecision::from_transport(
            cli::RemoteTransport::Exec,
            "cli",
            "explicit emergency compatibility",
        );

        assert_eq!(decision.selected_transport, "ssh-exec");
        assert!(decision.requires_external_ssh);
    }

    #[test]
    fn transport_metadata_classifies_direct_and_ssh_paths() {
        assert_eq!(
            direct_transport_policy(cli::RemoteTransport::TlsTcp),
            json!("production_direct")
        );
        assert_eq!(
            direct_transport_policy(cli::RemoteTransport::PlainTcp),
            json!("lab_baseline")
        );
        assert_eq!(
            tls_peer_auth_mode(
                cli::RemoteTransport::TlsTcp,
                Some("client.crt"),
                Some("client.key")
            ),
            json!("mutual_tls")
        );
        assert_eq!(
            ssh_mode_name(cli::RemoteTransport::Tcp),
            json!("spx-over-ssh-direct")
        );
        assert_eq!(
            ssh_data_plane_reason(cli::RemoteTransport::SshNative, Some("profile")),
            json!("explicit_user_choice")
        );
        assert_eq!(
            ssh_data_plane_reason(cli::RemoteTransport::Exec, None),
            json!("ssh_exec_compatibility")
        );
    }

    #[test]
    fn transport_selection_keeps_single_precedence_chain() {
        let defaults = config::ProxyProfile::default();
        let mut args = route_args();
        args.remote_transport = cli::RemoteTransport::Exec;

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, true)
                .expect("explicit transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Exec);
        assert_eq!(selected.source, "cli");

        let args = route_args();
        let profile = config::ProxyProfile {
            remote_transport: Some("tcp".to_string()),
            ..Default::default()
        };
        let selected = transport_selection_policy(
            &args,
            Some(&profile),
            &defaults,
            Some("192.0.2.8:19083".parse().unwrap()),
            Some("192.0.2.8:19082".parse().unwrap()),
            false,
            false,
            true,
        )
        .expect("profile transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Tcp);
        assert_eq!(selected.source, "profile");

        let selected = transport_selection_policy(
            &args,
            None,
            &defaults,
            Some("192.0.2.8:19083".parse().unwrap()),
            Some("192.0.2.8:19082".parse().unwrap()),
            false,
            false,
            true,
        )
        .expect("tls transport");
        assert_eq!(selected.transport, cli::RemoteTransport::TlsTcp);
        assert_eq!(selected.source, "topology");

        let selected = transport_selection_policy(
            &args,
            None,
            &defaults,
            Some("192.0.2.8:19083".parse().unwrap()),
            None,
            false,
            false,
            true,
        )
        .expect("quic transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Quic);

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, true)
                .expect("persistent peer transport");
        assert_eq!(selected.transport, cli::RemoteTransport::Tcp);
        assert_eq!(selected.source, "peer-default");

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, false)
                .expect("ssh-only transport");
        assert_eq!(selected.transport, cli::RemoteTransport::SshNative);
        assert_eq!(selected.source, "topology");
    }

    #[test]
    fn plain_tcp_default_needs_explicit_trust_source() {
        let args = route_args();
        let mut defaults = config::ProxyProfile {
            remote_transport: Some("plain-tcp".to_string()),
            ..Default::default()
        };

        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, false, false, false)
                .expect("plain tcp disabled");
        assert_eq!(selected.transport, cli::RemoteTransport::SshNative);

        defaults.allow_plain_tcp = Some(true);
        let selected =
            transport_selection_policy(&args, None, &defaults, None, None, true, false, false)
                .expect("plain tcp enabled");
        assert_eq!(selected.transport, cli::RemoteTransport::PlainTcp);
        assert_eq!(selected.source, "benchmark-tuned default");
    }
}
