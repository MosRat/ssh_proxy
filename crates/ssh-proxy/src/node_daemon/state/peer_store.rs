use anyhow::Result;
use serde_json::{Value, json};

use super::{PeerStatusRecord, ProductionState, STORE_VERSION, file_store::save_store};

impl ProductionState {
    pub(in crate::node_daemon) async fn peers_value(&self) -> Value {
        let peers = self
            .peers
            .lock()
            .await
            .peers
            .values()
            .map(|peer| serde_json::to_value(peer).unwrap_or_else(|_| json!({})))
            .collect::<Vec<_>>();
        json!(peers)
    }

    pub(in crate::node_daemon) async fn upsert_peer_status(
        &self,
        record: PeerStatusRecord,
    ) -> Result<()> {
        let mut store = self.peers.lock().await;
        store.version = STORE_VERSION;
        store.peers.insert(record.target.clone(), record);
        save_store(&self._peers_path, &*store)
    }

    pub(in crate::node_daemon) async fn peer_status(
        &self,
        target: &str,
    ) -> Option<PeerStatusRecord> {
        self.peers.lock().await.peers.get(target).cloned()
    }
}
