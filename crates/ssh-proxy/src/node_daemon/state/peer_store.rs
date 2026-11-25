use anyhow::Result;
use serde_json::{Value, json};

use super::{PeerStatusRecord, ProductionState, STORE_VERSION};

impl ProductionState {
    pub(in crate::node_daemon) async fn peers_value(&self) -> Value {
        let peers = self
            .peers
            .read()
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
        self.peers
            .update(|store| {
                store.version = STORE_VERSION;
                store.peers.insert(record.target.clone(), record);
            })
            .await
    }

    pub(in crate::node_daemon) async fn peer_status(
        &self,
        target: &str,
    ) -> Option<PeerStatusRecord> {
        self.peers.read().await.peers.get(target).cloned()
    }
}
