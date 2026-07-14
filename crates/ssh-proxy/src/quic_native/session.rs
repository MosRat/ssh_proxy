#![allow(dead_code)]

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use super::control::{RouteControlFrame, RouteControlHello, RouteControlWelcome};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSessionSpec {
    pub route_id: String,
    pub node: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub preferred_protocols: Vec<String>,
}

impl RouteSessionSpec {
    pub fn new(
        route_id: impl Into<String>,
        node: impl Into<String>,
        features: Vec<String>,
        preferred_protocols: Vec<String>,
    ) -> Self {
        Self {
            route_id: route_id.into(),
            node: node.into(),
            features,
            preferred_protocols,
        }
    }

    fn into_hello(self) -> RouteControlHello {
        RouteControlHello {
            version: super::control::CONTROL_FRAME_VERSION,
            route_id: self.route_id,
            node: self.node,
            features: self.features,
            preferred_protocols: self.preferred_protocols,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSessionWelcome {
    pub hello: RouteControlHello,
    pub welcome: RouteControlWelcome,
}

pub async fn client_negotiate<S>(
    stream: &mut S,
    spec: RouteSessionSpec,
) -> Result<RouteSessionWelcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello = spec.into_hello();
    RouteControlFrame::Hello(hello.clone())
        .write_to(stream)
        .await
        .context("failed to send QUIC-native route hello")?;
    let frame = RouteControlFrame::read_from(stream)
        .await
        .context("failed to read QUIC-native route welcome")?;
    let RouteControlFrame::Welcome(welcome) = frame else {
        bail!("expected QUIC-native route welcome frame");
    };
    if welcome.route_id != hello.route_id {
        bail!(
            "QUIC-native welcome route id mismatch: expected {}, got {}",
            hello.route_id,
            welcome.route_id
        );
    }
    if !welcome.accepted {
        bail!(
            "QUIC-native route session was rejected by remote: {}",
            welcome.message
        );
    }
    Ok(RouteSessionWelcome { hello, welcome })
}

pub async fn server_accept<S, F>(stream: &mut S, respond: F) -> Result<RouteControlHello>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(&RouteControlHello) -> RouteControlWelcome,
{
    let frame = RouteControlFrame::read_from(stream)
        .await
        .context("failed to read QUIC-native route hello")?;
    let RouteControlFrame::Hello(hello) = frame else {
        bail!("expected QUIC-native route hello frame");
    };
    let welcome = respond(&hello);
    if welcome.route_id != hello.route_id {
        bail!(
            "QUIC-native welcome route id mismatch: expected {}, got {}",
            hello.route_id,
            welcome.route_id
        );
    }
    RouteControlFrame::Welcome(welcome)
        .write_to(stream)
        .await
        .context("failed to send QUIC-native route welcome")?;
    Ok(hello)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn client_and_server_negotiate_route_session() {
        let (mut client, mut server) = duplex(16 * 1024);
        let spec = RouteSessionSpec::new(
            "route-1",
            "client-node",
            vec!["ssh-native-direct-tcpip".to_string()],
            vec!["quic-native".to_string()],
        );

        let server_task = tokio::spawn(async move {
            server_accept(&mut server, |hello| RouteControlWelcome {
                version: super::super::control::CONTROL_FRAME_VERSION,
                route_id: hello.route_id.clone(),
                accepted: true,
                selected_protocol: Some("quic-native".to_string()),
                message: "ok".to_string(),
            })
            .await
        });

        let welcome = client_negotiate(&mut client, spec).await.unwrap();
        let hello = server_task.await.unwrap().unwrap();

        assert_eq!(hello.route_id, "route-1");
        assert!(welcome.welcome.accepted);
        assert_eq!(
            welcome.welcome.selected_protocol.as_deref(),
            Some("quic-native")
        );
    }

    #[tokio::test]
    async fn client_rejects_unaccepted_route_session() {
        let (mut client, mut server) = duplex(16 * 1024);
        let spec = RouteSessionSpec::new(
            "route-1",
            "client-node",
            vec!["quic-native-streams-v1".to_string()],
            vec!["quic-native".to_string()],
        );

        let server_task = tokio::spawn(async move {
            server_accept(&mut server, |hello| RouteControlWelcome {
                version: super::super::control::CONTROL_FRAME_VERSION,
                route_id: hello.route_id.clone(),
                accepted: false,
                selected_protocol: Some("quic-native".to_string()),
                message: "policy rejected".to_string(),
            })
            .await
        });

        let err = client_negotiate(&mut client, spec).await.unwrap_err();
        server_task.await.unwrap().unwrap();

        assert!(err.to_string().contains("policy rejected"), "{err}");
    }
}
