use serde_json::json;

use crate::{protocol_core::report::RuntimeDecisionReport, route::RouteRuntimeDecision};

use super::RouteSpec;

impl RouteSpec {
    pub(super) fn direction(&self) -> &'static str {
        match self {
            Self::Forward { .. } => "forward",
            Self::Reverse { .. } => "reverse",
        }
    }

    pub(super) fn detail(&self) -> String {
        match self {
            Self::Forward { proxy } => format!("{} -> {}", proxy.listen, proxy.target),
            Self::Reverse { reverse } => format!("{} <- {}", reverse.remote_listen, reverse.target),
        }
    }

    pub(super) fn listen(&self) -> std::net::SocketAddr {
        match self {
            Self::Forward { proxy } => proxy.listen,
            Self::Reverse { reverse } => reverse.remote_listen,
        }
    }

    pub(super) fn peer(&self) -> &str {
        match self {
            Self::Forward { proxy } => &proxy.target,
            Self::Reverse { reverse } => &reverse.target,
        }
    }

    pub(in crate::node_daemon) fn runtime_metadata(&self) -> serde_json::Value {
        match self {
            Self::Forward { proxy } => RouteRuntimeDecision::from_forward_task(proxy).into_value(),
            Self::Reverse { reverse } => json!({
                "selected_transport": "ssh-reverse-link",
                "connection_decision": RuntimeDecisionReport::new(
                    "ssh-reverse-link",
                    "topology",
                    "remote-uses-local reverse-link uses the SSH-established reverse channel",
                ),
                "transport_pool_size": 1,
                "transport_pool_source": reverse.transport_pool_source.as_deref().unwrap_or("fixed"),
                "transport_pool_reason": reverse.transport_pool_reason.as_deref().unwrap_or("reverse-link currently uses one SSH-established route link"),
                "connect_timeout_secs": reverse.connect_timeout_secs,
                "reconnect_delay_secs": reverse.reconnect_delay_secs,
                "reconnect_max_delay_secs": reverse.reconnect_max_delay_secs,
                "no_reconnect": reverse.no_reconnect,
            }),
        }
    }
}
