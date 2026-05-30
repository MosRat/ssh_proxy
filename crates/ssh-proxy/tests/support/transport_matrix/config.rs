use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const GATE_ENV: &str = "SSH_PROXY_MATRIX";
const LEVEL_ENV: &str = "SSH_PROXY_MATRIX_LEVEL";
const TARGETS_ENV: &str = "SSH_PROXY_MATRIX_TARGETS";
const JUMP_TARGET_ENV: &str = "SSH_PROXY_MATRIX_JUMP_TARGET";
const DIRECT_TARGET_ENV: &str = "SSH_PROXY_MATRIX_DIRECT_TARGET";
const ACCEPT_NEW_ENV: &str = "SSH_PROXY_MATRIX_ACCEPT_NEW";
const KEEP_ENV: &str = "SSH_PROXY_MATRIX_KEEP";
const ARTIFACT_DIR_ENV: &str = "SSH_PROXY_MATRIX_ARTIFACT_DIR";
const LOCAL_BIN_ENV: &str = "SSH_PROXY_MATRIX_LOCAL_BIN";
const SIDECAR_ENV: &str = "SSH_PROXY_MATRIX_SIDECAR";
const DURATION_SECS_ENV: &str = "SSH_PROXY_MATRIX_DURATION_SECS";
const SAMPLES_ENV: &str = "SSH_PROXY_MATRIX_SAMPLES";
const CONCURRENCY_ENV: &str = "SSH_PROXY_MATRIX_CONCURRENCY";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MatrixLevel {
    Probe,
    Smoke,
    PerfSmoke,
    Stability,
}

#[derive(Debug, Clone)]
pub(super) struct MatrixConfig {
    pub(super) run_level: MatrixLevel,
    pub(super) requested: MatrixLevel,
    pub(super) targets: Vec<String>,
    pub(super) jump_target: Option<String>,
    pub(super) direct_target: Option<String>,
    pub(super) accept_new: bool,
    pub(super) keep: bool,
    pub(super) artifact_dir: PathBuf,
    pub(super) local_bin: PathBuf,
    pub(super) sidecar: PathBuf,
    pub(super) duration_secs: u64,
    pub(super) samples: usize,
    pub(super) concurrency: usize,
}

impl MatrixConfig {
    pub(super) fn load(requested: MatrixLevel) -> Option<Self> {
        if !env_flag(GATE_ENV) {
            eprintln!("skipping transport matrix: set {GATE_ENV}=1 to enable");
            return None;
        }

        let run_level = env::var(LEVEL_ENV)
            .ok()
            .and_then(|value| MatrixLevel::parse(&value))
            .unwrap_or(requested);
        let jump_target = env_string(JUMP_TARGET_ENV);
        let direct_target = env_string(DIRECT_TARGET_ENV);
        let targets = configured_targets(jump_target.as_deref(), direct_target.as_deref());
        if targets.is_empty() {
            eprintln!(
                "skipping transport matrix: set {TARGETS_ENV} or {JUMP_TARGET_ENV}/{DIRECT_TARGET_ENV}"
            );
            return None;
        }

        let artifact_dir = env_path(ARTIFACT_DIR_ENV).unwrap_or_else(default_artifact_dir);
        fs::create_dir_all(&artifact_dir).unwrap_or_else(|err| {
            panic!(
                "failed to create transport matrix artifact dir {}: {err}",
                artifact_dir.display()
            )
        });

        Some(Self {
            run_level,
            requested,
            targets,
            jump_target,
            direct_target,
            accept_new: env_flag(ACCEPT_NEW_ENV),
            keep: env_flag(KEEP_ENV),
            artifact_dir,
            local_bin: env_path(LOCAL_BIN_ENV).unwrap_or_else(default_local_bin),
            sidecar: env_path(SIDECAR_ENV).unwrap_or_else(default_sidecar),
            duration_secs: env_u64(DURATION_SECS_ENV).unwrap_or(match requested {
                MatrixLevel::Stability => 1800,
                MatrixLevel::PerfSmoke => 30,
                _ => 0,
            }),
            samples: env_usize(SAMPLES_ENV).unwrap_or(match requested {
                MatrixLevel::PerfSmoke => 4,
                MatrixLevel::Stability => 0,
                _ => 1,
            }),
            concurrency: env_usize(CONCURRENCY_ENV).unwrap_or(2),
        })
    }

    pub(super) fn should_run(&self, requested: MatrixLevel) -> bool {
        if self.run_level < requested {
            eprintln!(
                "skipping transport matrix {requested:?}: configured {LEVEL_ENV}={:?}",
                self.run_level
            );
            return false;
        }
        true
    }

    pub(super) fn topology_for(&self, target: &str) -> &'static str {
        if self.jump_target.as_deref() == Some(target) {
            "proxy_jump_no_login"
        } else if self.direct_target.as_deref() == Some(target) {
            "direct"
        } else {
            "declared"
        }
    }

    pub(super) fn is_direct_target(&self, target: &str) -> bool {
        self.direct_target.as_deref() == Some(target)
    }

    pub(super) fn level_name(&self) -> &'static str {
        self.requested.as_str()
    }
}

impl MatrixLevel {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "probe" => Some(Self::Probe),
            "smoke" => Some(Self::Smoke),
            "perf-smoke" | "perf_smoke" => Some(Self::PerfSmoke),
            "stability" => Some(Self::Stability),
            _ => None,
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Smoke => "smoke",
            Self::PerfSmoke => "perf-smoke",
            Self::Stability => "stability",
        }
    }
}

fn configured_targets(jump_target: Option<&str>, direct_target: Option<&str>) -> Vec<String> {
    let mut targets = BTreeSet::new();
    if let Some(value) = env_string(TARGETS_ENV) {
        for target in value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            targets.insert(target.to_string());
        }
    }
    if let Some(target) = jump_target {
        targets.insert(target.to_string());
    }
    if let Some(target) = direct_target {
        targets.insert(target.to_string());
    }
    targets.into_iter().collect()
}

pub(super) fn env_string(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env_string(name).map(PathBuf::from)
}

fn env_u64(name: &str) -> Option<u64> {
    env_string(name).and_then(|value| value.parse().ok())
}

fn env_usize(name: &str) -> Option<usize> {
    env_string(name).and_then(|value| value.parse().ok())
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn default_artifact_dir() -> PathBuf {
    std::env::temp_dir().join(format!("ssh_proxy-transport-matrix-{}", stamp()))
}

fn default_local_bin() -> PathBuf {
    workspace_root()
        .join("target")
        .join("release")
        .join(format!("ssh_proxy{}", std::env::consts::EXE_SUFFIX))
}

fn default_sidecar() -> PathBuf {
    workspace_root()
        .join("target")
        .join("x86_64-unknown-linux-musl")
        .join("release")
        .join("ssh_proxy")
}

pub(super) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("ssh_proxy package should live under crates/ssh-proxy")
        .to_path_buf()
}

pub(super) fn stamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    format!("{millis}-{}", std::process::id())
}
