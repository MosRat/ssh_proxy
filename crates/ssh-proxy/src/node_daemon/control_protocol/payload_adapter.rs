use anyhow::Result;
use ssh_proxy_daemon::control::{NodeRequestIntent, NodeRequestPayload};

use crate::protocol_core::control::{DaemonControlCommand, DaemonControlPayloadShape};

use super::{NODE_CONTROL_VERSION, NodeRequest};

impl NodeRequest {
    pub(crate) fn validate_compatible(&self) -> Result<()> {
        if let Some(version) = self.api_version
            && version > NODE_CONTROL_VERSION
        {
            anyhow::bail!(
                "unsupported node control api_version {version}; local daemon supports {NODE_CONTROL_VERSION}"
            );
        }
        Ok(())
    }

    pub(crate) fn command_kind(&self) -> DaemonControlCommand {
        DaemonControlCommand::parse(&self.cmd)
    }

    pub(crate) fn payload_shape(&self) -> DaemonControlPayloadShape {
        self.command_kind().payload_shape()
    }

    pub(crate) fn typed_payload(&self) -> NodeRequestPayload {
        match self.payload_shape() {
            DaemonControlPayloadShape::Empty => NodeRequestPayload::Empty,
            DaemonControlPayloadShape::Profile => NodeRequestPayload::Profile {
                profile: self.profile.clone(),
            },
            DaemonControlPayloadShape::Id => NodeRequestPayload::Id {
                id: self.id.clone(),
            },
            DaemonControlPayloadShape::RouteStart => NodeRequestPayload::RouteStart {
                id: self.id.clone(),
                direction: self.direction.clone(),
                persist: self.persist,
                has_proxy: self.proxy.is_some(),
                has_reverse: self.reverse.is_some(),
                connect_mode: self.connect_mode.clone(),
            },
            DaemonControlPayloadShape::RouteArgs => NodeRequestPayload::RouteArgs {
                has_route: self.route.is_some(),
            },
            DaemonControlPayloadShape::PeerBootstrap => NodeRequestPayload::PeerBootstrap {
                has_bootstrap: self.bootstrap.is_some(),
            },
            DaemonControlPayloadShape::Report => NodeRequestPayload::Report {
                node: self.node.clone(),
                has_status: self.status.is_some(),
            },
            DaemonControlPayloadShape::ProxySession => NodeRequestPayload::ProxySession {
                id: self.id.clone(),
                has_spec: self.proxy_session.is_some(),
            },
            DaemonControlPayloadShape::RemoteSettings => NodeRequestPayload::RemoteSettings {
                target: self.alias.clone(),
                workspace: self.id.clone(),
                remote_url: self.remote_url.clone(),
            },
            DaemonControlPayloadShape::DaemonUpdate => NodeRequestPayload::DaemonUpdate {
                source: self.update_source.clone(),
            },
            DaemonControlPayloadShape::JobEvents => NodeRequestPayload::JobEvents {
                id: self.id.clone(),
            },
            DaemonControlPayloadShape::Unknown => NodeRequestPayload::Unknown,
        }
    }

    pub(crate) fn typed_intent(&self) -> NodeRequestIntent {
        let command = self.command_kind();
        NodeRequestIntent::new(
            command.canonical_name(),
            self.api_version,
            self.id.clone(),
            self.alias.clone().or_else(|| self.node.clone()),
            self.typed_payload(),
        )
    }

    #[cfg(test)]
    pub(crate) fn typed_view(&self) -> ssh_proxy_daemon::control::NodeRequestView {
        self.typed_intent().view()
    }
}
