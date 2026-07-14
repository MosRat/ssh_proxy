use std::sync::Arc;

use tokio::{
    sync::Mutex,
    task::JoinHandle,
    time::{self, Duration},
};
use tracing::{error, warn};

use crate::{config, controller, reverse};

use super::{RouteLinkState, RouteSpec, RouteStats, time_util::now_unix};

pub(super) fn spawn_route_supervisor(
    id: String,
    spec: RouteSpec,
    config: config::AppConfig,
    stats: Arc<Mutex<RouteStats>>,
    link_state: Option<RouteLinkState>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut delay = Duration::from_secs(1);
        loop {
            {
                let mut stats = stats.lock().await;
                stats.state = "running".to_string();
                stats.attempts += 1;
                stats.last_event = Some("route task starting".to_string());
                stats.updated_at_unix = now_unix();
            }
            let direction = spec.direction();
            let result = match spec.clone() {
                RouteSpec::Forward { proxy } => match link_state.clone() {
                    Some(RouteLinkState::Spx(state)) => {
                        controller::run_with_state(proxy, state).await
                    }
                    Some(RouteLinkState::QuicNative(state)) => {
                        controller::run_quic_native_with_slot(proxy, state).await
                    }
                    Some(RouteLinkState::SshNative(state)) => {
                        controller::run_ssh_native_with_state(proxy, state).await
                    }
                    None => controller::run(proxy).await,
                },
                RouteSpec::Reverse { reverse } => {
                    reverse::run(reverse.into(), config.clone()).await
                }
            };
            let mut stats = stats.lock().await;
            stats.updated_at_unix = now_unix();
            match result {
                Ok(()) => {
                    stats.state = "exited".to_string();
                    stats.last_event = Some("route task exited cleanly".to_string());
                    warn!(route = %id, %direction, "route task exited cleanly; restarting");
                }
                Err(err) => {
                    stats.state = "error".to_string();
                    stats.last_error = Some(err.to_string());
                    stats.last_event = Some("route task failed".to_string());
                    error!(route = %id, %direction, error = %err, "route task failed; restarting");
                }
            }
            stats.restart_count += 1;
            stats.state = "restarting".to_string();
            stats.last_event = Some(format!("route task restarting in {}s", delay.as_secs()));
            stats.updated_at_unix = now_unix();
            drop(stats);
            time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(30));
        }
    })
}
