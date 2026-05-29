use std::{
    fmt,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::time;

use crate::{config, ssh_client};

use super::{proxy_session::ProxySessionSpec, remote_ssh};

const SOURCE: &str = "rust_ssh_direct_tcpip";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const INITIAL_DELAY: Duration = Duration::from_millis(250);
const MAX_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct HandoffProbeStatus {
    pub(super) source: String,
    pub(super) state: String,
    pub(super) attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) retry_after_ms: Option<u64>,
    pub(super) updated_at_unix: u64,
}

impl HandoffProbeStatus {
    pub(super) fn checking() -> Self {
        Self {
            source: SOURCE.to_string(),
            state: "checking".to_string(),
            attempts: 0,
            last_error: None,
            latency_ms: None,
            retry_after_ms: Some(INITIAL_DELAY.as_millis() as u64),
            updated_at_unix: now_unix(),
        }
    }

    pub(super) fn skipped() -> Self {
        Self {
            source: SOURCE.to_string(),
            state: "skipped".to_string(),
            attempts: 0,
            last_error: None,
            latency_ms: None,
            retry_after_ms: None,
            updated_at_unix: now_unix(),
        }
    }

    fn ready(attempts: u32, latency: Duration) -> Self {
        Self {
            source: SOURCE.to_string(),
            state: "ready".to_string(),
            attempts,
            last_error: None,
            latency_ms: Some(latency.as_millis() as u64),
            retry_after_ms: None,
            updated_at_unix: now_unix(),
        }
    }

    fn failed(attempts: u32, error: String, latency: Option<Duration>) -> Self {
        Self {
            source: SOURCE.to_string(),
            state: "failed".to_string(),
            attempts,
            last_error: Some(error),
            latency_ms: latency.map(|value| value.as_millis() as u64),
            retry_after_ms: None,
            updated_at_unix: now_unix(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct HandoffProbeFailure {
    pub(super) blocker: String,
    pub(super) message: String,
    pub(super) status: HandoffProbeStatus,
}

impl fmt::Display for HandoffProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HandoffProbeFailure {}

pub(super) async fn wait_remote_port_ready(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
    budget: Duration,
) -> Result<HandoffProbeStatus, HandoffProbeFailure> {
    let host = spec.remote_bind.to_string();
    let port = spec.remote_port_policy.preferred;
    let install_args = remote_ssh::install_args_from_spec(config, spec).map_err(|err| {
        failure(
            "ssh_direct_tcpip_failed",
            format!(
                "failed to build SSH handoff target for {}: {err}",
                spec.target
            ),
            0,
            None,
        )
    })?;
    let client = match time::timeout(
        CONNECT_TIMEOUT.min(budget.max(Duration::from_millis(1))),
        ssh_client::Client::connect_install_args(&install_args),
    )
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(err)) => {
            return Err(failure(
                "ssh_direct_tcpip_failed",
                format!("failed to connect SSH target for handoff probe: {err}"),
                0,
                None,
            ));
        }
        Err(_) => {
            return Err(failure(
                "ssh_direct_tcpip_failed",
                format!(
                    "timed out connecting SSH target for handoff probe after {CONNECT_TIMEOUT:?}"
                ),
                0,
                None,
            ));
        }
    };

    let started = Instant::now();
    let mut attempts = 0;
    let mut delay = INITIAL_DELAY;

    loop {
        attempts += 1;
        let attempt_started = Instant::now();
        let (last_error, last_latency) = match time::timeout(
            PROBE_TIMEOUT,
            client.direct_tcpip_stream(host.clone(), port),
        )
        .await
        {
            Ok(Ok(stream)) => {
                drop(stream);
                return Ok(HandoffProbeStatus::ready(
                    attempts,
                    attempt_started.elapsed(),
                ));
            }
            Ok(Err(err)) => (err.to_string(), Some(attempt_started.elapsed())),
            Err(_) => (
                format!(
                    "remote port {host}:{port} did not accept direct-tcpip within {PROBE_TIMEOUT:?}",
                ),
                Some(attempt_started.elapsed()),
            ),
        };

        if started.elapsed() >= budget {
            let blocker = match classify_probe_error(&last_error) {
                "remote_port_not_ready" => "handoff_timeout",
                other => other,
            };
            let detail = format!(
                "handoff timed out after {} attempts probing {host}:{port}: {last_error}",
                attempts
            );
            return Err(failure(blocker, detail, attempts, last_latency));
        }

        let remaining = budget.saturating_sub(started.elapsed());
        time::sleep(delay.min(remaining)).await;
        delay = (delay * 2).min(MAX_DELAY);
    }
}

pub(super) fn classify_probe_error(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("refused") {
        "remote_port_refused"
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "remote_port_not_ready"
    } else {
        "ssh_direct_tcpip_failed"
    }
}

fn failure(
    blocker: impl Into<String>,
    message: String,
    attempts: u32,
    latency: Option<Duration>,
) -> HandoffProbeFailure {
    let blocker = blocker.into();
    HandoffProbeFailure {
        blocker: blocker.clone(),
        status: HandoffProbeStatus::failed(attempts, message.clone(), latency),
        message,
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_error_classification_is_stable() {
        assert_eq!(
            classify_probe_error("Connection refused while opening direct-tcpip"),
            "remote_port_refused"
        );
        assert_eq!(
            classify_probe_error("operation timed out"),
            "remote_port_not_ready"
        );
        assert_eq!(
            classify_probe_error("administratively prohibited"),
            "ssh_direct_tcpip_failed"
        );
    }

    #[test]
    fn probe_status_shapes_are_json_stable() {
        let checking = serde_json::to_value(HandoffProbeStatus::checking()).unwrap();
        assert_eq!(checking["source"], SOURCE);
        assert_eq!(checking["state"], "checking");
        assert_eq!(checking["attempts"], 0);
        assert_eq!(checking["retry_after_ms"], INITIAL_DELAY.as_millis() as u64);

        let skipped = serde_json::to_value(HandoffProbeStatus::skipped()).unwrap();
        assert_eq!(skipped["state"], "skipped");
        assert!(skipped.get("retry_after_ms").is_none());
    }
}
