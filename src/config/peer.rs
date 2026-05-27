use super::{AppConfig, PeerRecord, now_unix};

impl AppConfig {
    pub fn record_peer(&mut self, alias: &str, mut peer: PeerRecord) {
        if peer.transport_protocols.is_empty() {
            peer.transport_protocols = peer.known_transport_protocols();
        }
        peer.last_seen_unix = Some(now_unix());
        self.peers.insert(alias.to_string(), peer);
    }
}

pub(super) fn sorted_peers(config: &AppConfig) -> Vec<(&String, &PeerRecord)> {
    let mut peers = config.peers.iter().collect::<Vec<_>>();
    peers.sort_by(|(left, _), (right, _)| left.cmp(right));
    peers
}
