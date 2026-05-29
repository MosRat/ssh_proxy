use serde_json::Value;

use crate::service::{
    inventory::{ServiceProbeState, ServiceProbeSummary},
    plan::ServiceScope,
};

pub(super) fn service_probe_summary(
    scope: ServiceScope,
    service_name: String,
    state: ServiceProbeState,
    exists: bool,
    healthy: bool,
    accessible: bool,
    permission_denied: bool,
    details: Value,
) -> ServiceProbeSummary {
    ServiceProbeSummary {
        scope,
        service_name,
        state,
        exists,
        healthy,
        accessible,
        permission_denied,
        details,
    }
}

pub(super) fn contains_permission_denied(text: &str) -> bool {
    text.to_ascii_lowercase().contains("access is denied")
        || text.to_ascii_lowercase().contains("permission denied")
        || text.to_ascii_lowercase().contains("not permitted")
        || text
            .to_ascii_lowercase()
            .contains("operation not permitted")
}
