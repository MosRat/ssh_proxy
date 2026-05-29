use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRequestKind {
    Connect,
    Disconnect,
    Status,
    Descriptor,
    Links,
    Shutdown,
    Nodes,
    Jobs,
    JobStatus,
    JobEvents,
    NodeEnsure,
    NodeStart,
    NodeStop,
    NodeRestart,
    RouteIntent,
    RoutePlan,
    RouteStart,
    RouteStop,
    RouteRestart,
    RouteList,
    PeerList,
    TokenRotate,
    RemotePeerEnsure,
    RemotePeerStatus,
    RemotePeerRepair,
    RemotePeerUpdate,
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
    DaemonUpdate,
    Unknown,
}

impl NodeRequestKind {
    pub fn from_command(command: &str) -> Self {
        let normalized = normalize_command(command);
        match normalized.as_str() {
            "connect" => Self::Connect,
            "disconnect" => Self::Disconnect,
            "" | "status" => Self::Status,
            "descriptor" | "describe" => Self::Descriptor,
            "links" => Self::Links,
            "shutdown" => Self::Shutdown,
            "nodes" | "node_list" => Self::Nodes,
            "jobs" | "job_list" => Self::Jobs,
            "job_status" => Self::JobStatus,
            "job_events" => Self::JobEvents,
            "node_ensure" => Self::NodeEnsure,
            "node_start" => Self::NodeStart,
            "node_stop" => Self::NodeStop,
            "node_restart" => Self::NodeRestart,
            "route_intent" => Self::RouteIntent,
            "route_plan" => Self::RoutePlan,
            "route_start" => Self::RouteStart,
            "route_stop" => Self::RouteStop,
            "route_restart" => Self::RouteRestart,
            "route_list" | "routes" => Self::RouteList,
            "peer_list" | "peers" => Self::PeerList,
            "token_rotate" => Self::TokenRotate,
            "remote_peer_ensure" => Self::RemotePeerEnsure,
            "remote_peer_status" => Self::RemotePeerStatus,
            "remote_peer_repair" => Self::RemotePeerRepair,
            "remote_peer_update" => Self::RemotePeerUpdate,
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
            "daemon_update" => Self::DaemonUpdate,
            _ => Self::Unknown,
        }
    }
}

fn normalize_command(command: &str) -> String {
    command.trim().to_ascii_lowercase().replace('-', "_")
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
            NodeRequestKind::from_command("node-list"),
            NodeRequestKind::Nodes
        );
        assert_eq!(
            NodeRequestKind::from_command("routes"),
            NodeRequestKind::RouteList
        );
        assert_eq!(
            NodeRequestKind::from_command("new_command"),
            NodeRequestKind::Unknown
        );
    }
}
