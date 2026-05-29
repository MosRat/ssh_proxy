use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) const CONTROL_API_VERSION: u16 = 1;
pub(crate) const PEER_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ControlApiVersion(u16);

impl ControlApiVersion {
    pub(crate) const fn current() -> Self {
        Self(CONTROL_API_VERSION)
    }

    pub(crate) const fn new(version: u16) -> Self {
        Self(version)
    }

    pub(crate) const fn value(self) -> u16 {
        self.0
    }

    pub(crate) const fn is_supported_by(self, supported: Self) -> bool {
        self.0 <= supported.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct PeerProtocolVersion(u16);

impl PeerProtocolVersion {
    pub(crate) const fn current() -> Self {
        Self(PEER_PROTOCOL_VERSION)
    }

    pub(crate) const fn new(version: u16) -> Self {
        Self(version)
    }

    pub(crate) const fn value(self) -> u16 {
        self.0
    }

    pub(crate) const fn matches_required(self, required: Self) -> bool {
        self.0 == required.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FeatureSet {
    pub(crate) features: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(crate) feature_bits: Map<String, Value>,
}

impl FeatureSet {
    pub(crate) fn new(features: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let features = features.into_iter().map(Into::into).collect::<Vec<_>>();
        let feature_bits = features
            .iter()
            .cloned()
            .map(|feature| (feature, Value::Bool(true)))
            .collect();
        Self {
            features,
            feature_bits,
        }
    }

    pub(crate) fn from_parts(features: Vec<String>, feature_bits: Map<String, Value>) -> Self {
        Self {
            features,
            feature_bits,
        }
    }

    pub(crate) fn missing_from(&self, remote: &Self) -> Vec<String> {
        let remote = remote.features.iter().collect::<BTreeSet<_>>();
        self.features
            .iter()
            .filter(|feature| !remote.contains(feature))
            .cloned()
            .collect()
    }

    pub(crate) fn common_with(&self, remote: &Self) -> Vec<String> {
        let remote = remote.features.iter().collect::<BTreeSet<_>>();
        self.features
            .iter()
            .filter(|feature| remote.contains(feature))
            .cloned()
            .collect()
    }

    pub(crate) fn supports_all(&self, required: &Self) -> bool {
        required.missing_from(self).is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VersionCompatibility {
    Compatible,
    UpgradeLocal,
    UpgradeRemote,
    Incompatible(String),
}

pub(crate) fn classify_protocol_compatibility(
    local_control: ControlApiVersion,
    remote_control: Option<ControlApiVersion>,
    local_peer: PeerProtocolVersion,
    remote_peer: Option<PeerProtocolVersion>,
    local_features: &FeatureSet,
    remote_features: &FeatureSet,
) -> VersionCompatibility {
    if remote_control.is_some_and(|remote| !remote.is_supported_by(local_control)) {
        return VersionCompatibility::UpgradeLocal;
    }
    if remote_peer.is_some_and(|remote| remote.value() > local_peer.value()) {
        return VersionCompatibility::UpgradeLocal;
    }
    if remote_control.is_none() || remote_peer.is_none() {
        return VersionCompatibility::UpgradeRemote;
    }
    if remote_peer.is_some_and(|remote| !remote.matches_required(local_peer)) {
        return VersionCompatibility::UpgradeRemote;
    }
    let missing = local_features.missing_from(remote_features);
    if !missing.is_empty() {
        return VersionCompatibility::Incompatible(format!(
            "remote is missing required features: {}",
            missing.join(", ")
        ));
    }
    VersionCompatibility::Compatible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_set_reports_common_and_missing_features() {
        let local = FeatureSet::new(["frames-v1", "quic-native-streams-v1"]);
        let remote = FeatureSet::new(["frames-v1"]);

        assert_eq!(
            local.missing_from(&remote),
            vec!["quic-native-streams-v1".to_string()]
        );
        assert_eq!(local.common_with(&remote), vec!["frames-v1".to_string()]);
        assert!(!remote.supports_all(&local));
    }

    #[test]
    fn compatibility_classifies_future_versions() {
        let features = FeatureSet::new(["frames-v1"]);

        assert_eq!(
            classify_protocol_compatibility(
                ControlApiVersion::current(),
                Some(ControlApiVersion::new(CONTROL_API_VERSION + 1)),
                PeerProtocolVersion::current(),
                Some(PeerProtocolVersion::current()),
                &features,
                &features,
            ),
            VersionCompatibility::UpgradeLocal
        );
        assert_eq!(
            classify_protocol_compatibility(
                ControlApiVersion::current(),
                Some(ControlApiVersion::current()),
                PeerProtocolVersion::current(),
                Some(PeerProtocolVersion::new(PEER_PROTOCOL_VERSION + 1)),
                &features,
                &features,
            ),
            VersionCompatibility::UpgradeLocal
        );
    }

    #[test]
    fn compatibility_classifies_missing_versions_and_features() {
        let features = FeatureSet::new(["frames-v1"]);
        let missing_features = FeatureSet::new(["other"]);

        assert_eq!(
            classify_protocol_compatibility(
                ControlApiVersion::current(),
                None,
                PeerProtocolVersion::current(),
                Some(PeerProtocolVersion::current()),
                &features,
                &features,
            ),
            VersionCompatibility::UpgradeRemote
        );
        assert!(matches!(
            classify_protocol_compatibility(
                ControlApiVersion::current(),
                Some(ControlApiVersion::current()),
                PeerProtocolVersion::current(),
                Some(PeerProtocolVersion::current()),
                &features,
                &missing_features,
            ),
            VersionCompatibility::Incompatible(_)
        ));
    }
}
