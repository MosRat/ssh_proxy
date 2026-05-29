use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const CONTROL_API_VERSION: u16 = 1;
pub const PEER_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ControlApiVersion(u16);

impl ControlApiVersion {
    pub const fn current() -> Self {
        Self(CONTROL_API_VERSION)
    }

    pub const fn new(version: u16) -> Self {
        Self(version)
    }

    pub const fn value(self) -> u16 {
        self.0
    }

    pub const fn is_supported_by(self, supported: Self) -> bool {
        self.0 <= supported.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerProtocolVersion(u16);

impl PeerProtocolVersion {
    pub const fn current() -> Self {
        Self(PEER_PROTOCOL_VERSION)
    }

    pub const fn new(version: u16) -> Self {
        Self(version)
    }

    pub const fn value(self) -> u16 {
        self.0
    }

    pub const fn matches_required(self, required: Self) -> bool {
        self.0 == required.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSet {
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub feature_bits: Map<String, Value>,
}

impl FeatureSet {
    pub fn new(features: impl IntoIterator<Item = impl Into<String>>) -> Self {
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

    pub fn from_parts(features: Vec<String>, feature_bits: Map<String, Value>) -> Self {
        Self {
            features,
            feature_bits,
        }
    }

    pub fn missing_from(&self, remote: &Self) -> Vec<String> {
        let remote = remote.features.iter().collect::<BTreeSet<_>>();
        self.features
            .iter()
            .filter(|feature| !remote.contains(feature))
            .cloned()
            .collect()
    }

    pub fn common_with(&self, remote: &Self) -> Vec<String> {
        let remote = remote.features.iter().collect::<BTreeSet<_>>();
        self.features
            .iter()
            .filter(|feature| remote.contains(feature))
            .cloned()
            .collect()
    }

    pub fn supports_all(&self, required: &Self) -> bool {
        required.missing_from(self).is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionCompatibility {
    Compatible,
    UpgradeLocal,
    UpgradeRemote,
    Incompatible(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolCompatibilityReport {
    pub checks: Vec<Value>,
    pub compatible: bool,
    pub missing_features: Vec<String>,
    pub common_features: Vec<String>,
}

pub fn protocol_compatibility_report(
    local_control: u16,
    remote_control: Option<u16>,
    local_peer: u16,
    remote_peer: Option<u16>,
    local_features: &[String],
    remote_features: &[String],
) -> ProtocolCompatibilityReport {
    let local_feature_set = FeatureSet::new(local_features.iter().cloned());
    let remote_feature_set = FeatureSet::new(remote_features.iter().cloned());
    let missing_features = local_feature_set.missing_from(&remote_feature_set);
    let common_features = local_feature_set.common_with(&remote_feature_set);
    let checks = vec![
        control_api_check(local_control, remote_control),
        peer_protocol_check(local_peer, remote_peer),
        feature_check(local_features, remote_features, &missing_features),
    ];
    let compatible = checks
        .iter()
        .all(|check| check.get("severity").and_then(Value::as_str) != Some("error"));
    ProtocolCompatibilityReport {
        checks,
        compatible,
        missing_features,
        common_features,
    }
}

fn control_api_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote <= local => json!({
            "name": "control_api_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote control API is supported"
        }),
        Some(remote) => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote control API is newer than this binary supports"
        }),
        None => json!({
            "name": "control_api_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "remote descriptor does not advertise a control API version"
        }),
    }
}

fn peer_protocol_check(local: u16, remote: Option<u16>) -> Value {
    match remote {
        Some(remote) if remote == local => json!({
            "name": "peer_protocol_version",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote peer data protocol matches"
        }),
        Some(remote) if remote > local => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote peer data protocol is newer than this binary supports"
        }),
        Some(remote) => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote peer data protocol is older than this binary requires"
        }),
        None => json!({
            "name": "peer_protocol_version",
            "ok": false,
            "local": local,
            "remote": Value::Null,
            "severity": "error",
            "message": "remote descriptor does not advertise a peer data protocol version"
        }),
    }
}

fn feature_check(local: &[String], remote: &[String], missing: &[String]) -> Value {
    if missing.is_empty() {
        json!({
            "name": "features",
            "ok": true,
            "local": local,
            "remote": remote,
            "severity": "info",
            "message": "remote advertises all locally required data-plane features"
        })
    } else {
        json!({
            "name": "features",
            "ok": false,
            "local": local,
            "remote": remote,
            "severity": "error",
            "message": "remote is missing required data-plane features",
            "missing": missing,
        })
    }
}

pub fn classify_protocol_compatibility(
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

pub fn compare_dotted_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let left = parse_dotted_version(left)?;
    let right = parse_dotted_version(right)?;
    Some(left.cmp(&right))
}

fn parse_dotted_version(value: &str) -> Option<Vec<u64>> {
    let core = value.split_once('-').map(|(core, _)| core).unwrap_or(value);
    let parts = core
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (!parts.is_empty()).then_some(parts)
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

    #[test]
    fn protocol_report_centralizes_checks_and_features() {
        let local = vec!["frames-v1".to_string(), "tcp-connect".to_string()];
        let remote = vec!["frames-v1".to_string()];
        let report = protocol_compatibility_report(1, Some(1), 1, Some(1), &local, &remote);

        assert!(!report.compatible);
        assert_eq!(report.common_features, vec!["frames-v1".to_string()]);
        assert_eq!(report.missing_features, vec!["tcp-connect".to_string()]);
        assert_eq!(report.checks[2]["name"], "features");
        assert_eq!(report.checks[2]["severity"], "error");
    }
}
