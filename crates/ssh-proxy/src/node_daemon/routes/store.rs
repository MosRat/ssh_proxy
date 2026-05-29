use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config;

use super::{NodeManager, RouteSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredRoute {
    id: String,
    created_at_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fallback_reason: Option<String>,
    spec: RouteSpec,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RouteStore {
    version: u32,
    routes: Vec<StoredRoute>,
}

impl NodeManager {
    pub(in crate::node_daemon) async fn save_routes(&self) -> Result<()> {
        let routes = self.routes.lock().await;
        let store = RouteStore {
            version: 1,
            routes: routes
                .iter()
                .filter(|(_, task)| task.persist)
                .map(|(id, task)| StoredRoute {
                    id: id.clone(),
                    created_at_unix: task.created_at_unix,
                    fallback_reason: task.fallback_reason.clone(),
                    spec: task.spec.clone(),
                })
                .collect(),
        };
        drop(routes);
        let text = serde_json::to_string_pretty(&store)?;
        config::save_text_file_private(&self.route_store_path, &text).with_context(|| {
            format!(
                "failed to write persistent route store {}",
                self.route_store_path.display()
            )
        })?;
        Ok(())
    }

    pub(in crate::node_daemon) async fn restore_routes(&self) {
        let path = self.route_store_path.clone();
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                info!(path = %path.display(), "no persistent route store found");
                return;
            }
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to read persistent route store");
                return;
            }
        };
        let store: RouteStore = match serde_json::from_str(&text) {
            Ok(store) => store,
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to parse persistent route store");
                return;
            }
        };
        for route in store.routes {
            let id = route.id.clone();
            match self
                .start_route_spec(
                    route.id,
                    route.spec,
                    true,
                    route.created_at_unix,
                    route.fallback_reason,
                )
                .await
            {
                Ok(_) => info!(route = %id, "restored persistent route"),
                Err(err) => warn!(route = %id, error = %err, "failed to restore persistent route"),
            }
        }
    }
}
