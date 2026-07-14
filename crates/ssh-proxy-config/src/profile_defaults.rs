use std::{path::PathBuf, str::FromStr};

use anyhow::{Result, anyhow};
use ssh_proxy_core::{
    intent::{DeploymentPolicy, QuicRuntimeTuningIntent, RouteEndpointIntent, RuntimeTuningIntent},
    model::{RemotePlatform, TcpTarget, TransportMode},
};

use crate::{schema::ProxyProfile, store::expand_path};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileIntentDefaults {
    pub target: Option<String>,
    pub listen: Option<std::net::SocketAddr>,
    pub tcp_target: Option<TcpTarget>,
    pub ssh_args: Vec<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity: Vec<PathBuf>,
    pub config: Option<PathBuf>,
    pub known_hosts: Option<PathBuf>,
    pub accept_new: Option<bool>,
    pub insecure_ignore_host_key: Option<bool>,
    pub jump: Vec<String>,
    pub remote_path: Option<String>,
    pub remote_bin: Option<PathBuf>,
    pub deployment: Option<DeploymentPolicy>,
    pub remote_platform: Option<RemotePlatform>,
    pub transport: Option<TransportMode>,
    pub endpoint: RouteEndpointIntent,
    pub remote_token: Option<String>,
    pub runtime: RuntimeTuningIntent,
    pub transport_pool_size: Option<usize>,
    pub ssh_session_pool_size: Option<usize>,
    pub allow_plain_tcp: Option<bool>,
    pub no_reconnect: Option<bool>,
}

pub fn plan_profile_defaults(profile: &ProxyProfile) -> Result<ProfileIntentDefaults> {
    Ok(ProfileIntentDefaults {
        target: profile.target.clone(),
        listen: profile.listen,
        tcp_target: profile.tcp_target.clone(),
        ssh_args: profile.ssh_args.clone(),
        user: profile.user.clone(),
        port: profile.port,
        identity: expand_paths(&profile.identity),
        config: profile.config.as_ref().map(expand_path),
        known_hosts: profile.known_hosts.as_ref().map(expand_path),
        accept_new: profile.accept_new,
        insecure_ignore_host_key: profile.insecure_ignore_host_key,
        jump: profile.jump.clone(),
        remote_path: profile.remote_path.clone(),
        remote_bin: profile.remote_bin.as_ref().map(expand_path),
        deployment: profile
            .deploy
            .as_deref()
            .map(parse_deployment_policy)
            .transpose()?,
        remote_platform: profile
            .remote_os
            .as_deref()
            .map(parse_remote_platform)
            .transpose()?,
        transport: profile
            .remote_transport
            .as_deref()
            .map(parse_transport_mode)
            .transpose()?,
        endpoint: RouteEndpointIntent {
            listen: profile.listen,
            control_listen: profile.control_listen,
            tcp_target: profile.tcp_target.clone(),
            remote_tcp: profile.remote_tcp,
            remote_control: profile.remote_control,
            remote_quic: profile.remote_quic,
            remote_tls: profile.remote_tls,
            remote_name: profile.remote_name.clone(),
            remote_ca: profile.remote_ca.as_ref().map(expand_path),
            remote_client_cert: profile.remote_client_cert.as_ref().map(expand_path),
            remote_client_key: profile.remote_client_key.as_ref().map(expand_path),
            egress_proxy: profile.egress_proxy.clone(),
            allow_plain_tcp: profile.allow_plain_tcp.unwrap_or(false),
            ..Default::default()
        },
        remote_token: profile.remote_token.clone(),
        runtime: RuntimeTuningIntent {
            reconnect_delay_secs: profile.reconnect_delay_secs,
            reconnect_max_delay_secs: profile.reconnect_max_delay_secs,
            connect_timeout_secs: profile.connect_timeout_secs,
            transport_pool_size: profile.transport_pool_size,
            ssh_session_pool_size: profile.ssh_session_pool_size,
            workload_hint: profile.workload_hint,
            quic: QuicRuntimeTuningIntent {
                max_bidi_streams: profile.quic_max_bidi_streams,
                stream_receive_window: profile.quic_stream_receive_window,
                receive_window: profile.quic_receive_window,
                keep_alive_interval_secs: profile.quic_keep_alive_interval_secs,
                idle_timeout_secs: profile.quic_idle_timeout_secs,
            },
            no_reconnect: profile.no_reconnect.unwrap_or(false),
        },
        transport_pool_size: profile.transport_pool_size,
        ssh_session_pool_size: profile.ssh_session_pool_size,
        allow_plain_tcp: profile.allow_plain_tcp,
        no_reconnect: profile.no_reconnect,
    })
}

pub fn parse_deployment_policy(value: &str) -> Result<DeploymentPolicy> {
    DeploymentPolicy::from_str(value).map_err(|_| invalid("deploy", value))
}

pub fn parse_remote_platform(value: &str) -> Result<RemotePlatform> {
    RemotePlatform::from_str(value).map_err(|_| invalid("remote_os", value))
}

pub fn parse_transport_mode(value: &str) -> Result<TransportMode> {
    TransportMode::from_str(value).map_err(|_| invalid("remote_transport", value))
}

fn invalid(field: &str, value: &str) -> anyhow::Error {
    anyhow!("invalid {field} value {value:?}")
}

fn expand_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths.iter().map(expand_path).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ssh_proxy_core::model::WorkloadHint;

    use super::*;

    #[test]
    fn profile_defaults_parse_command_neutral_modes() {
        let profile = ProxyProfile {
            deploy: Some("always".to_string()),
            remote_os: Some("windows".to_string()),
            remote_transport: Some("tls_tcp".to_string()),
            workload_hint: Some(WorkloadHint::Concurrent),
            transport_pool_size: Some(4),
            ..Default::default()
        };

        let defaults = plan_profile_defaults(&profile).unwrap();

        assert_eq!(defaults.deployment, Some(DeploymentPolicy::Always));
        assert_eq!(defaults.remote_platform, Some(RemotePlatform::Windows));
        assert_eq!(defaults.transport, Some(TransportMode::TlsTcp));
        assert_eq!(
            defaults.runtime.workload_hint,
            Some(WorkloadHint::Concurrent)
        );
        assert_eq!(defaults.transport_pool_size, Some(4));
    }

    #[test]
    fn profile_defaults_expand_path_fields_once() {
        let profile = ProxyProfile {
            identity: vec![PathBuf::from("id_ed25519")],
            remote_ca: Some(PathBuf::from("ca.pem")),
            remote_client_cert: Some(PathBuf::from("client.pem")),
            remote_client_key: Some(PathBuf::from("client-key.pem")),
            ..Default::default()
        };

        let defaults = plan_profile_defaults(&profile).unwrap();

        assert_eq!(defaults.identity, vec![PathBuf::from("id_ed25519")]);
        assert_eq!(
            defaults.endpoint.remote_ca.as_deref(),
            Some(Path::new("ca.pem"))
        );
        assert_eq!(
            defaults.endpoint.remote_client_cert.as_deref(),
            Some(Path::new("client.pem"))
        );
        assert_eq!(
            defaults.endpoint.remote_client_key.as_deref(),
            Some(Path::new("client-key.pem"))
        );
    }

    #[test]
    fn profile_defaults_reject_invalid_modes() {
        let profile = ProxyProfile {
            remote_transport: Some("shell-magic".to_string()),
            ..Default::default()
        };

        let err = plan_profile_defaults(&profile).unwrap_err().to_string();

        assert!(err.contains("invalid remote_transport value"));
    }
}
