use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod status;

pub use status::{
    ServiceManagerSummaryInput, control_endpoint_kind_from_str, persistent_manager_kind,
    selected_control_summary, service_candidates_summary, service_manager_summary,
    service_next_action, service_state_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceScope {
    User,
    System,
}

impl ServiceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceScope::User => "user",
            ServiceScope::System => "system",
        }
    }
}

impl fmt::Display for ServiceScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ServiceScope {
    type Err = ParseServiceScopeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(ServiceScope::User),
            "system" => Ok(ServiceScope::System),
            _ => Err(ParseServiceScopeError {
                value: value.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedServiceScope {
    Auto,
    User,
    System,
}

impl RequestedServiceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestedServiceScope::Auto => "auto",
            RequestedServiceScope::User => "user",
            RequestedServiceScope::System => "system",
        }
    }
}

impl fmt::Display for RequestedServiceScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RequestedServiceScope {
    type Err = ParseServiceScopeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(RequestedServiceScope::Auto),
            "user" => Ok(RequestedServiceScope::User),
            "system" => Ok(RequestedServiceScope::System),
            _ => Err(ParseServiceScopeError {
                value: value.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseServiceScopeError {
    value: String,
}

impl fmt::Display for ParseServiceScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported service scope `{}`", self.value)
    }
}

impl std::error::Error for ParseServiceScopeError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceProbeState {
    Healthy,
    Present,
    Missing,
    PermissionDenied,
    Unknown,
}

impl ServiceProbeState {
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceProbeState::Healthy => "healthy",
            ServiceProbeState::Present => "present",
            ServiceProbeState::Missing => "missing",
            ServiceProbeState::PermissionDenied => "permission_denied",
            ServiceProbeState::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProbeSummary {
    pub scope: ServiceScope,
    pub service_name: String,
    pub state: ServiceProbeState,
    pub exists: bool,
    pub healthy: bool,
    pub accessible: bool,
    pub permission_denied: bool,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProbePlan {
    pub scope: ServiceScope,
    pub service_name: String,
    pub provider: String,
    pub command: Vec<String>,
}

impl ServiceProbePlan {
    pub fn new(
        scope: ServiceScope,
        service_name: impl Into<String>,
        provider: impl Into<String>,
        command: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            scope,
            service_name: service_name.into(),
            provider: provider.into(),
            command: command.into_iter().map(Into::into).collect(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "scope": self.scope.as_str(),
            "service_name": self.service_name,
            "provider": self.provider,
            "command": self.command,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProviderReport {
    pub provider: String,
    pub probe: ServiceProbeSummary,
}

impl ServiceProviderReport {
    pub fn new(provider: impl Into<String>, probe: ServiceProbeSummary) -> Self {
        Self {
            provider: provider.into(),
            probe,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "provider": self.provider,
            "probe": self.probe.to_json(),
        })
    }
}

impl ServiceProbeSummary {
    pub fn to_json(&self) -> Value {
        json!({
            "scope": self.scope.as_str(),
            "service_name": self.service_name,
            "state": self.state.as_str(),
            "exists": self.exists,
            "healthy": self.healthy,
            "accessible": self.accessible,
            "permission_denied": self.permission_denied,
            "details": self.details.clone(),
        })
    }
}

pub fn service_probe_summary(
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

pub fn contains_permission_denied(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("not permitted")
        || lower.contains("operation not permitted")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceNextAction {
    Reuse,
    StartOrRepair,
    Install,
    Unavailable,
}

impl ServiceNextAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceNextAction::Reuse => "reuse",
            ServiceNextAction::StartOrRepair => "start_or_repair",
            ServiceNextAction::Install => "install",
            ServiceNextAction::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInventory {
    pub requested_scope: RequestedServiceScope,
    pub selected_scope: Option<ServiceScope>,
    pub preferred_install_scope: Option<ServiceScope>,
    pub fallback_chain: Vec<ServiceScope>,
    pub probe_chain: Vec<ServiceProbeSummary>,
    pub selected_reason: String,
    pub next_action: ServiceNextAction,
}

impl ServiceInventory {
    pub fn to_json(&self) -> Value {
        json!({
            "requested_scope": self.requested_scope.as_str(),
            "selected_scope": self.selected_scope.map(ServiceScope::as_str),
            "preferred_install_scope": self.preferred_install_scope.map(ServiceScope::as_str),
            "fallback_chain": self.fallback_chain.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
            "selected_reason": self.selected_reason,
            "next_action": self.next_action.as_str(),
            "probe_chain": self.probe_chain.iter().map(ServiceProbeSummary::to_json).collect::<Vec<_>>(),
        })
    }
}

pub fn resolve_service_inventory(
    requested_scope: RequestedServiceScope,
    preferred_install_scope: ServiceScope,
    probe_chain: Vec<ServiceProbeSummary>,
) -> ServiceInventory {
    let fallback_chain = requested_fallback_chain(requested_scope);
    let preferred_install_scope = Some(match requested_scope {
        RequestedServiceScope::Auto => preferred_install_scope,
        RequestedServiceScope::User => ServiceScope::User,
        RequestedServiceScope::System => ServiceScope::System,
    });

    let selected_scope = select_scope(requested_scope, preferred_install_scope, &probe_chain);
    let selected_reason = selected_reason(requested_scope, selected_scope, &probe_chain);
    let next_action = next_action(selected_scope, &probe_chain);

    ServiceInventory {
        requested_scope,
        selected_scope,
        preferred_install_scope,
        fallback_chain,
        probe_chain,
        selected_reason,
        next_action,
    }
}

fn requested_fallback_chain(requested_scope: RequestedServiceScope) -> Vec<ServiceScope> {
    match requested_scope {
        RequestedServiceScope::Auto => vec![ServiceScope::System, ServiceScope::User],
        RequestedServiceScope::User => vec![ServiceScope::User],
        RequestedServiceScope::System => vec![ServiceScope::System],
    }
}

fn select_scope(
    requested_scope: RequestedServiceScope,
    preferred_install_scope: Option<ServiceScope>,
    probe_chain: &[ServiceProbeSummary],
) -> Option<ServiceScope> {
    if matches!(requested_scope, RequestedServiceScope::Auto) {
        for scope in [ServiceScope::System, ServiceScope::User] {
            if probe_for_scope(probe_chain, scope).is_some_and(|probe| probe.exists) {
                return Some(scope);
            }
        }
        return preferred_install_scope;
    }

    Some(match requested_scope {
        RequestedServiceScope::User => ServiceScope::User,
        RequestedServiceScope::System => ServiceScope::System,
        RequestedServiceScope::Auto => preferred_install_scope?,
    })
}

fn selected_reason(
    requested_scope: RequestedServiceScope,
    selected_scope: Option<ServiceScope>,
    probe_chain: &[ServiceProbeSummary],
) -> String {
    let Some(scope) = selected_scope else {
        return "no persistent service scope could be selected".to_string();
    };
    let Some(probe) = probe_for_scope(probe_chain, scope) else {
        return format!("selected {scope:?} scope because it matches the requested preference");
    };

    if probe.healthy {
        return format!("selected existing healthy {:?} service", scope);
    }
    if probe.exists {
        return format!(
            "selected existing {:?} service for repair or restart",
            scope
        );
    }

    match requested_scope {
        RequestedServiceScope::Auto => format!(
            "no persistent service was found; selected {:?} as the install target",
            scope
        ),
        RequestedServiceScope::User => "user scope was requested explicitly".to_string(),
        RequestedServiceScope::System => "system scope was requested explicitly".to_string(),
    }
}

fn next_action(
    selected_scope: Option<ServiceScope>,
    probe_chain: &[ServiceProbeSummary],
) -> ServiceNextAction {
    let Some(scope) = selected_scope else {
        return ServiceNextAction::Unavailable;
    };
    match probe_for_scope(probe_chain, scope) {
        Some(probe) if probe.healthy => ServiceNextAction::Reuse,
        Some(probe) if probe.exists => ServiceNextAction::StartOrRepair,
        Some(_) => ServiceNextAction::Install,
        None => ServiceNextAction::Unavailable,
    }
}

fn probe_for_scope(
    probe_chain: &[ServiceProbeSummary],
    scope: ServiceScope,
) -> Option<&ServiceProbeSummary> {
    probe_chain.iter().find(|probe| probe.scope == scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(scope: ServiceScope, exists: bool, healthy: bool) -> ServiceProbeSummary {
        ServiceProbeSummary {
            scope,
            service_name: format!("{scope:?}"),
            state: if healthy {
                ServiceProbeState::Healthy
            } else if exists {
                ServiceProbeState::Present
            } else {
                ServiceProbeState::Missing
            },
            exists,
            healthy,
            accessible: true,
            permission_denied: false,
            details: json!({}),
        }
    }

    #[test]
    fn auto_prefers_highest_existing_scope() {
        let inventory = resolve_service_inventory(
            RequestedServiceScope::Auto,
            ServiceScope::User,
            vec![
                probe(ServiceScope::System, true, false),
                probe(ServiceScope::User, true, true),
            ],
        );

        assert_eq!(inventory.selected_scope, Some(ServiceScope::System));
        assert_eq!(inventory.next_action, ServiceNextAction::StartOrRepair);
        assert!(inventory.selected_reason.contains("repair"));
    }

    #[test]
    fn auto_falls_back_to_install_scope_when_no_service_exists() {
        let inventory = resolve_service_inventory(
            RequestedServiceScope::Auto,
            ServiceScope::User,
            vec![
                probe(ServiceScope::System, false, false),
                probe(ServiceScope::User, false, false),
            ],
        );

        assert_eq!(inventory.selected_scope, Some(ServiceScope::User));
        assert_eq!(inventory.next_action, ServiceNextAction::Install);
    }

    #[test]
    fn explicit_scope_is_kept_even_when_others_exist() {
        let inventory = resolve_service_inventory(
            RequestedServiceScope::System,
            ServiceScope::User,
            vec![
                probe(ServiceScope::System, false, false),
                probe(ServiceScope::User, true, true),
            ],
        );

        assert_eq!(inventory.selected_scope, Some(ServiceScope::System));
        assert_eq!(inventory.next_action, ServiceNextAction::Install);
    }

    #[test]
    fn inventory_json_preserves_public_shape() {
        let inventory = resolve_service_inventory(
            RequestedServiceScope::Auto,
            ServiceScope::User,
            vec![probe(ServiceScope::User, true, true)],
        );
        let value = inventory.to_json();

        assert_eq!(value["requested_scope"], "auto");
        assert_eq!(value["selected_scope"], "user");
        assert_eq!(value["next_action"], "reuse");
        assert_eq!(value["probe_chain"][0]["state"], "healthy");
    }

    #[test]
    fn probe_plan_and_provider_report_render_stable_json() {
        let plan = ServiceProbePlan::new(
            ServiceScope::User,
            "ssh_proxy",
            "systemd",
            ["systemctl", "--user", "status", "ssh_proxy.service"],
        );
        let report = ServiceProviderReport::new(
            "systemd",
            service_probe_summary(
                ServiceScope::User,
                "ssh_proxy".to_string(),
                ServiceProbeState::Present,
                true,
                false,
                true,
                false,
                json!({"capture": plan.to_json()}),
            ),
        );
        let value = report.to_json();

        assert_eq!(value["provider"], "systemd");
        assert_eq!(value["probe"]["scope"], "user");
        assert_eq!(value["probe"]["details"]["capture"]["provider"], "systemd");
    }

    #[test]
    fn permission_denied_classifier_matches_platform_messages() {
        assert!(contains_permission_denied("Access is denied."));
        assert!(contains_permission_denied("operation not permitted"));
        assert!(!contains_permission_denied("service is inactive"));
    }
}
