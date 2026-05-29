use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRequestKind {
    Connect,
    Disconnect,
    Status,
    Descriptor,
    RouteIntent,
    RoutePlan,
    RouteStart,
    RouteStop,
    RouteRestart,
    Routes,
    PeerBootstrap,
    PeerEnsure,
    PeerUpdate,
    PeerRefresh,
    PeerDiff,
    PeerReconcile,
    PeerCheckVersion,
    PeerRotateToken,
    PeerForget,
    Report,
    EnsureProxySession,
    ProxySessionStatus,
    ProxySessionDown,
    ApplyRemoteSettings,
    RemotePeerEnsure,
    RemotePeerStatus,
    DaemonUpdate,
    JobEvents,
    Unknown,
}

impl NodeRequestKind {
    pub fn from_command(command: &str) -> Self {
        match command {
            "connect" => Self::Connect,
            "disconnect" => Self::Disconnect,
            "status" => Self::Status,
            "descriptor" => Self::Descriptor,
            "route_intent" => Self::RouteIntent,
            "route_plan" => Self::RoutePlan,
            "route_start" => Self::RouteStart,
            "route_stop" => Self::RouteStop,
            "route_restart" => Self::RouteRestart,
            "routes" => Self::Routes,
            "peer_bootstrap" => Self::PeerBootstrap,
            "peer_ensure" => Self::PeerEnsure,
            "peer_update" => Self::PeerUpdate,
            "peer_refresh" => Self::PeerRefresh,
            "peer_diff" => Self::PeerDiff,
            "peer_reconcile" => Self::PeerReconcile,
            "peer_check_version" => Self::PeerCheckVersion,
            "peer_rotate_token" => Self::PeerRotateToken,
            "peer_forget" => Self::PeerForget,
            "report" => Self::Report,
            "ensure_proxy_session" => Self::EnsureProxySession,
            "proxy_session_status" => Self::ProxySessionStatus,
            "proxy_session_down" => Self::ProxySessionDown,
            "apply_remote_settings" => Self::ApplyRemoteSettings,
            "remote_peer_ensure" => Self::RemotePeerEnsure,
            "remote_peer_status" => Self::RemotePeerStatus,
            "daemon_update" => Self::DaemonUpdate,
            "job_events" => Self::JobEvents,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRequestView {
    pub kind: NodeRequestKind,
    pub command: String,
    pub api_version: Option<u16>,
    pub id: Option<String>,
    pub alias: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_request_kind_accepts_existing_commands() {
        assert_eq!(
            NodeRequestKind::from_command("ensure_proxy_session"),
            NodeRequestKind::EnsureProxySession
        );
        assert_eq!(
            NodeRequestKind::from_command("remote_peer_ensure"),
            NodeRequestKind::RemotePeerEnsure
        );
        assert_eq!(
            NodeRequestKind::from_command("new_command"),
            NodeRequestKind::Unknown
        );
    }
}
