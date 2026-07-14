use serde::{Deserialize, Serialize};
use ssh_proxy_protocol::protocol_core::control::{DaemonControlCommand, DaemonControlPayloadShape};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum NodeRequestPayload {
    Empty,
    Profile {
        profile: Option<String>,
    },
    Id {
        id: Option<String>,
    },
    RouteStart {
        id: Option<String>,
        direction: Option<String>,
        persist: Option<bool>,
        has_proxy: bool,
        has_reverse: bool,
        connect_mode: Option<String>,
    },
    RouteArgs {
        has_route: bool,
    },
    PeerBootstrap {
        has_bootstrap: bool,
    },
    Report {
        node: Option<String>,
        has_status: bool,
    },
    ProxySession {
        id: Option<String>,
        has_spec: bool,
    },
    RemoteSettings {
        target: Option<String>,
        workspace: Option<String>,
        remote_url: Option<String>,
    },
    DaemonUpdate {
        source: Option<String>,
    },
    JobEvents {
        id: Option<String>,
    },
    Unknown,
}

impl NodeRequestPayload {
    pub fn empty_for_shape(shape: DaemonControlPayloadShape) -> Self {
        match shape {
            DaemonControlPayloadShape::Empty => Self::Empty,
            DaemonControlPayloadShape::Profile => Self::Profile { profile: None },
            DaemonControlPayloadShape::Id => Self::Id { id: None },
            DaemonControlPayloadShape::RouteStart => Self::RouteStart {
                id: None,
                direction: None,
                persist: None,
                has_proxy: false,
                has_reverse: false,
                connect_mode: None,
            },
            DaemonControlPayloadShape::RouteArgs => Self::RouteArgs { has_route: false },
            DaemonControlPayloadShape::PeerBootstrap => Self::PeerBootstrap {
                has_bootstrap: false,
            },
            DaemonControlPayloadShape::Report => Self::Report {
                node: None,
                has_status: false,
            },
            DaemonControlPayloadShape::ProxySession => Self::ProxySession {
                id: None,
                has_spec: false,
            },
            DaemonControlPayloadShape::RemoteSettings => Self::RemoteSettings {
                target: None,
                workspace: None,
                remote_url: None,
            },
            DaemonControlPayloadShape::DaemonUpdate => Self::DaemonUpdate { source: None },
            DaemonControlPayloadShape::JobEvents => Self::JobEvents { id: None },
            DaemonControlPayloadShape::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRequestIntent {
    pub kind: NodeRequestKind,
    pub command: String,
    pub api_version: Option<u16>,
    pub id: Option<String>,
    pub alias: Option<String>,
    pub payload: NodeRequestPayload,
}

impl NodeRequestIntent {
    pub fn new(
        command: impl AsRef<str>,
        api_version: Option<u16>,
        id: Option<String>,
        alias: Option<String>,
        payload: NodeRequestPayload,
    ) -> Self {
        let command = DaemonControlCommand::parse(command.as_ref())
            .canonical_name()
            .to_string();
        Self {
            kind: NodeRequestKind::from_command(&command),
            command,
            api_version,
            id,
            alias,
            payload,
        }
    }

    pub fn view(&self) -> NodeRequestView {
        NodeRequestView {
            kind: self.kind,
            command: self.command.clone(),
            api_version: self.api_version,
            id: self.id.clone(),
            alias: self.alias.clone(),
        }
    }
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

    #[test]
    fn request_intent_preserves_legacy_route_start_payload() {
        let intent = NodeRequestIntent::new(
            "route-start",
            Some(1),
            Some("route-a".to_string()),
            None,
            NodeRequestPayload::RouteStart {
                id: Some("route-a".to_string()),
                direction: Some("forward".to_string()),
                persist: Some(true),
                has_proxy: true,
                has_reverse: false,
                connect_mode: None,
            },
        );

        assert_eq!(intent.kind, NodeRequestKind::RouteStart);
        assert_eq!(intent.command, "route_start");
        assert_eq!(intent.view().id.as_deref(), Some("route-a"));
    }

    #[test]
    fn empty_payload_matches_protocol_shape() {
        assert_eq!(
            NodeRequestPayload::empty_for_shape(DaemonControlPayloadShape::RemoteSettings),
            NodeRequestPayload::RemoteSettings {
                target: None,
                workspace: None,
                remote_url: None,
            }
        );
    }
}
