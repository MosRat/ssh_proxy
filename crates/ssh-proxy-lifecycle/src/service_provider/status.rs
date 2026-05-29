use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatusState {
    Healthy,
    Present,
    Missing,
    PermissionDenied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub state: ProviderStatusState,
    pub healthy: bool,
    pub message: String,
}

pub fn classify_provider_status(exit_status: u32, stdout: &str, stderr: &str) -> ProviderStatus {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let permission_denied = text.contains("access is denied")
        || text.contains("permission denied")
        || text.contains("not permitted")
        || text.contains("operation not permitted");
    let missing = text.contains("not-found")
        || text.contains("not found")
        || text.contains("could not be found")
        || text.contains("does not exist");
    let healthy = exit_status == 0
        && (text.contains("running")
            || text.contains("active")
            || text.contains("healthy")
            || text.contains("success"));
    let state = if permission_denied {
        ProviderStatusState::PermissionDenied
    } else if healthy {
        ProviderStatusState::Healthy
    } else if missing {
        ProviderStatusState::Missing
    } else if exit_status == 0 {
        ProviderStatusState::Present
    } else {
        ProviderStatusState::Unknown
    };
    ProviderStatus {
        state,
        healthy,
        message: if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        },
    }
}
