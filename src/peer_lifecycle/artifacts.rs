use super::config::PeerConfigFiles;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerArtifact {
    Config,
    PeerState,
    InstallReport,
    Health,
    Routes,
}

impl PeerArtifact {
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::Config => "config.toml",
            Self::PeerState => "peer_state.json",
            Self::InstallReport => "install_report.json",
            Self::Health => "health.json",
            Self::Routes => "routes.json",
        }
    }

    pub(crate) fn preserve_existing(self) -> bool {
        matches!(self, Self::Routes)
    }

    pub(crate) fn backup_existing(self) -> bool {
        matches!(self, Self::Config)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PeerArtifactBytes {
    pub(crate) artifact: PeerArtifact,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn materialized_peer_artifacts(files: PeerConfigFiles) -> Vec<PeerArtifactBytes> {
    vec![
        PeerArtifactBytes {
            artifact: PeerArtifact::Config,
            bytes: files.config_toml.into_bytes(),
        },
        PeerArtifactBytes {
            artifact: PeerArtifact::PeerState,
            bytes: files.peer_state_json.into_bytes(),
        },
        PeerArtifactBytes {
            artifact: PeerArtifact::InstallReport,
            bytes: files.install_report_json.into_bytes(),
        },
        PeerArtifactBytes {
            artifact: PeerArtifact::Health,
            bytes: files.health_json.into_bytes(),
        },
        PeerArtifactBytes {
            artifact: PeerArtifact::Routes,
            bytes: files.routes_json.into_bytes(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_names_match_remote_peer_files() {
        assert_eq!(PeerArtifact::Config.file_name(), "config.toml");
        assert_eq!(PeerArtifact::PeerState.file_name(), "peer_state.json");
        assert!(PeerArtifact::Routes.preserve_existing());
        assert!(PeerArtifact::Config.backup_existing());
    }
}
