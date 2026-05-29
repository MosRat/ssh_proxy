use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonControlCommand {
    Status,
    Descriptor,
    Links,
    Shutdown,
    Nodes,
    Jobs,
    JobStatus,
    JobEvents,
    EnsureProxySession,
    ProxySessionStatus,
    ProxySessionDown,
    DaemonUpdate,
    ApplyRemoteSettings,
    NodeEnsure,
    NodeStart,
    NodeStop,
    NodeRestart,
    Connect,
    Disconnect,
    RouteStart,
    RoutePlan,
    RouteIntent,
    RouteStop,
    RouteRestart,
    RouteList,
    PeerList,
    RemotePeerEnsure,
    RemotePeerStatus,
    RemotePeerRepair,
    RemotePeerUpdate,
    TokenRotate,
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
    Unknown(String),
}

impl DaemonControlCommand {
    pub(crate) fn parse(value: &str) -> Self {
        let lower = value.trim().to_ascii_lowercase();
        let normalized = normalize_command(&lower);
        match normalized.as_str() {
            "" | "status" => Self::Status,
            "descriptor" | "describe" => Self::Descriptor,
            "links" => Self::Links,
            "shutdown" => Self::Shutdown,
            "nodes" | "node_list" => Self::Nodes,
            "jobs" | "job_list" => Self::Jobs,
            "job_status" => Self::JobStatus,
            "job_events" => Self::JobEvents,
            "ensure_proxy_session" => Self::EnsureProxySession,
            "proxy_session_status" => Self::ProxySessionStatus,
            "proxy_session_down" => Self::ProxySessionDown,
            "daemon_update" => Self::DaemonUpdate,
            "apply_remote_settings" => Self::ApplyRemoteSettings,
            "node_ensure" => Self::NodeEnsure,
            "node_start" => Self::NodeStart,
            "node_stop" => Self::NodeStop,
            "node_restart" => Self::NodeRestart,
            "connect" => Self::Connect,
            "disconnect" => Self::Disconnect,
            "route_start" => Self::RouteStart,
            "route_plan" => Self::RoutePlan,
            "route_intent" => Self::RouteIntent,
            "route_stop" => Self::RouteStop,
            "route_restart" => Self::RouteRestart,
            "route_list" | "routes" => Self::RouteList,
            "peer_list" | "peers" => Self::PeerList,
            "remote_peer_ensure" => Self::RemotePeerEnsure,
            "remote_peer_status" => Self::RemotePeerStatus,
            "remote_peer_repair" => Self::RemotePeerRepair,
            "remote_peer_update" => Self::RemotePeerUpdate,
            "token_rotate" => Self::TokenRotate,
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
            _ => Self::Unknown(lower),
        }
    }

    pub(crate) fn canonical_name(&self) -> &str {
        match self {
            Self::Status => "status",
            Self::Descriptor => "descriptor",
            Self::Links => "links",
            Self::Shutdown => "shutdown",
            Self::Nodes => "nodes",
            Self::Jobs => "jobs",
            Self::JobStatus => "job_status",
            Self::JobEvents => "job_events",
            Self::EnsureProxySession => "ensure_proxy_session",
            Self::ProxySessionStatus => "proxy_session_status",
            Self::ProxySessionDown => "proxy_session_down",
            Self::DaemonUpdate => "daemon_update",
            Self::ApplyRemoteSettings => "apply_remote_settings",
            Self::NodeEnsure => "node_ensure",
            Self::NodeStart => "node_start",
            Self::NodeStop => "node_stop",
            Self::NodeRestart => "node_restart",
            Self::Connect => "connect",
            Self::Disconnect => "disconnect",
            Self::RouteStart => "route_start",
            Self::RoutePlan => "route_plan",
            Self::RouteIntent => "route_intent",
            Self::RouteStop => "route_stop",
            Self::RouteRestart => "route_restart",
            Self::RouteList => "route_list",
            Self::PeerList => "peer_list",
            Self::RemotePeerEnsure => "remote_peer_ensure",
            Self::RemotePeerStatus => "remote_peer_status",
            Self::RemotePeerRepair => "remote_peer_repair",
            Self::RemotePeerUpdate => "remote_peer_update",
            Self::TokenRotate => "token_rotate",
            Self::PeerBootstrap => "peer_bootstrap",
            Self::PeerEnsure => "peer_ensure",
            Self::PeerUpdate => "peer_update",
            Self::PeerRefresh => "peer_refresh",
            Self::PeerDiff => "peer_diff",
            Self::PeerReconcile => "peer_reconcile",
            Self::PeerCheckVersion => "peer_check_version",
            Self::PeerRotateToken => "peer_rotate_token",
            Self::PeerForget => "peer_forget",
            Self::Report => "report",
            Self::Unknown(value) => value.as_str(),
        }
    }

    pub(crate) fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }
}

pub(crate) fn normalize_command(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_command_parser_accepts_legacy_aliases() {
        assert_eq!(
            DaemonControlCommand::parse(""),
            DaemonControlCommand::Status
        );
        assert_eq!(
            DaemonControlCommand::parse("node-list"),
            DaemonControlCommand::Nodes
        );
        assert_eq!(
            DaemonControlCommand::parse("remote-peer-ensure"),
            DaemonControlCommand::RemotePeerEnsure
        );
        assert_eq!(
            DaemonControlCommand::parse("peer_check_version"),
            DaemonControlCommand::PeerCheckVersion
        );
    }

    #[test]
    fn daemon_command_parser_preserves_unknown_command() {
        let command = DaemonControlCommand::parse("Custom-Thing");

        assert_eq!(
            command,
            DaemonControlCommand::Unknown("custom-thing".to_string())
        );
        assert_eq!(command.canonical_name(), "custom-thing");
        assert!(command.is_unknown());
    }
}
