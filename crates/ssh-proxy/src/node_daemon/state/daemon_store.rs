use anyhow::Result;
use serde_json::{Value, json};

use super::{ProductionState, STORE_VERSION, now_unix};

impl ProductionState {
    pub(in crate::node_daemon) async fn record_daemon_started(
        &self,
        name: &str,
        control_endpoint: &str,
    ) -> Result<()> {
        self.daemon
            .update(|store| {
                let now = now_unix();
                store.version = STORE_VERSION;
                store.daemon.version = env!("CARGO_PKG_VERSION").to_string();
                store.daemon.health = "healthy".to_string();
                if matches!(
                    store.daemon.update_state.as_str(),
                    "switching" | "restart_daemon"
                ) {
                    store.daemon.update_state = "healthy".to_string();
                    let update = json!({
                        "state": "healthy",
                        "message": "daemon restarted after staged update",
                        "updated_at_unix": now,
                    });
                    store.daemon.update = Some(match store.daemon.update.take() {
                        Some(Value::Object(mut existing)) => {
                            if let Value::Object(update_object) = update {
                                existing.extend(update_object);
                            }
                            Value::Object(existing)
                        }
                        _ => update,
                    });
                } else {
                    store.daemon.update_state = "idle".to_string();
                }
                store.daemon.control_endpoint = Some(control_endpoint.to_string());
                store.daemon.name = Some(name.to_string());
                store.daemon.started_at_unix = now;
                store.daemon.updated_at_unix = now;
            })
            .await
    }

    pub(in crate::node_daemon) async fn record_daemon_update_requested(
        &self,
        source: Option<String>,
    ) -> Result<Value> {
        self.daemon
            .update(|store| {
                let now = now_unix();
                store.version = STORE_VERSION;
                store.daemon.version = env!("CARGO_PKG_VERSION").to_string();
                store.daemon.health = "healthy".to_string();
                store.daemon.update_state = "pending".to_string();
                store.daemon.updated_at_unix = now;
                store.daemon.update = Some(json!({
                    "state": store.daemon.update_state,
                    "source": source,
                    "updated_at_unix": now,
                }));
                store.daemon.update.clone().unwrap_or_else(|| {
                    json!({
                        "state": store.daemon.update_state,
                    })
                })
            })
            .await
    }

    pub(in crate::node_daemon) async fn record_daemon_update_state(
        &self,
        state: &str,
        source: Option<String>,
        staged_path: Option<String>,
        staged_hash: Option<String>,
        staged_version: Option<String>,
        switch_script: Option<String>,
        backup_path: Option<String>,
        last_error: Option<String>,
    ) -> Result<Value> {
        self.daemon
            .update(|store| {
                let now = now_unix();
                store.version = STORE_VERSION;
                store.daemon.version = env!("CARGO_PKG_VERSION").to_string();
                store.daemon.health = if state == "failed" {
                    "degraded".to_string()
                } else {
                    "healthy".to_string()
                };
                store.daemon.update_state = state.to_string();
                store.daemon.updated_at_unix = now;
                let update = json!({
                    "state": store.daemon.update_state,
                    "source": source,
                    "staged_path": staged_path,
                    "staged_hash": staged_hash,
                    "staged_version": staged_version,
                    "switch_script": switch_script,
                    "backup_path": backup_path,
                    "last_error": last_error,
                    "updated_at_unix": now,
                });
                store.daemon.update = Some(update.clone());
                update
            })
            .await
    }

    pub(in crate::node_daemon) async fn daemon_value(&self) -> Value {
        let store = self.daemon.read().await;
        serde_json::to_value(&store.daemon).unwrap_or_else(|_| json!({ "health": "unknown" }))
    }
}
