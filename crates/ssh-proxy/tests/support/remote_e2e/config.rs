use std::{collections::BTreeSet, env};

const E2E_GATE: &str = "SSH_PROXY_REMOTE_E2E";
const LEVEL_ENV: &str = "SSH_PROXY_REMOTE_LEVEL";
const TARGETS_ENV: &str = "SSH_PROXY_REMOTE_TARGETS";
const JUMP_TARGET_ENV: &str = "SSH_PROXY_REMOTE_JUMP_TARGET";
const DIRECT_TARGET_ENV: &str = "SSH_PROXY_REMOTE_DIRECT_TARGET";
const UPSTREAM_PROXY_ENV: &str = "SSH_PROXY_REMOTE_UPSTREAM_PROXY";
const ACCEPT_NEW_ENV: &str = "SSH_PROXY_REMOTE_ACCEPT_NEW";
const KEEP_ENV: &str = "SSH_PROXY_REMOTE_KEEP";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RemoteLevel {
    Probe,
    Smoke,
    Full,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteConfig {
    pub(super) run_level: RemoteLevel,
    pub(super) targets: Vec<String>,
    pub(super) jump_target: Option<String>,
    pub(super) direct_target: Option<String>,
    pub(super) upstream_proxy: Option<String>,
    pub(super) accept_new: bool,
    pub(super) keep: bool,
}

impl RemoteConfig {
    pub(super) fn load(requested: RemoteLevel) -> Option<Self> {
        if !env_flag(E2E_GATE) {
            eprintln!("skipping remote e2e: set {E2E_GATE}=1 to enable");
            return None;
        }

        let run_level = env::var(LEVEL_ENV)
            .ok()
            .and_then(|value| RemoteLevel::parse(&value))
            .unwrap_or(requested);
        let jump_target = env_string(JUMP_TARGET_ENV);
        let direct_target = env_string(DIRECT_TARGET_ENV);
        let targets = configured_targets(jump_target.as_deref(), direct_target.as_deref());
        if targets.is_empty() {
            eprintln!(
                "skipping remote e2e: set {TARGETS_ENV} or {JUMP_TARGET_ENV}/{DIRECT_TARGET_ENV}"
            );
            return None;
        }

        Some(Self {
            run_level,
            targets,
            jump_target,
            direct_target,
            upstream_proxy: env_string(UPSTREAM_PROXY_ENV),
            accept_new: env_flag(ACCEPT_NEW_ENV),
            keep: env_flag(KEEP_ENV),
        })
    }

    pub(super) fn run(&self, requested: RemoteLevel, test: impl FnOnce(&Self)) {
        if self.run_level < requested {
            eprintln!(
                "skipping remote {requested:?}: configured {LEVEL_ENV}={:?}",
                self.run_level
            );
            return;
        }
        test(self);
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
}

impl RemoteLevel {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "probe" => Some(Self::Probe),
            "smoke" => Some(Self::Smoke),
            "full" => Some(Self::Full),
            _ => None,
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

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}
