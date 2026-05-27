use serde_json::{Value, json};

use crate::cli;

use super::plan::{ServiceScope, preferred_install_scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceProbeState {
    Healthy,
    Present,
    Missing,
    PermissionDenied,
    Unknown,
}

#[derive(Debug, Clone)]
pub(crate) struct ServiceProbeSummary {
    pub(crate) scope: ServiceScope,
    pub(crate) service_name: String,
    pub(crate) state: ServiceProbeState,
    pub(crate) exists: bool,
    pub(crate) healthy: bool,
    pub(crate) accessible: bool,
    pub(crate) permission_denied: bool,
    pub(crate) details: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceNextAction {
    Reuse,
    StartOrRepair,
    Install,
    Unavailable,
}

impl ServiceNextAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ServiceNextAction::Reuse => "reuse",
            ServiceNextAction::StartOrRepair => "start_or_repair",
            ServiceNextAction::Install => "install",
            ServiceNextAction::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ServiceInventory {
    pub(crate) requested_scope: cli::ServiceScope,
    pub(crate) selected_scope: Option<ServiceScope>,
    pub(crate) preferred_install_scope: Option<ServiceScope>,
    pub(crate) fallback_chain: Vec<ServiceScope>,
    pub(crate) probe_chain: Vec<ServiceProbeSummary>,
    pub(crate) selected_reason: String,
    pub(crate) next_action: ServiceNextAction,
}

pub(crate) fn collect_service_inventory() -> Vec<ServiceProbeSummary> {
    let scopes = [ServiceScope::System, ServiceScope::User];
    scopes
        .into_iter()
        .map(super::platform::platform_probe_summary)
        .collect()
}

pub(crate) fn resolve_service_inventory(
    requested_scope: cli::ServiceScope,
    probe_chain: Vec<ServiceProbeSummary>,
) -> ServiceInventory {
    let fallback_chain = requested_fallback_chain(requested_scope);
    let preferred_install_scope = Some(match requested_scope {
        cli::ServiceScope::Auto => preferred_install_scope(),
        cli::ServiceScope::User => ServiceScope::User,
        cli::ServiceScope::System => ServiceScope::System,
    });

    let selected_scope = select_scope(requested_scope, &probe_chain);
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

pub(crate) fn inventory_json(inventory: &ServiceInventory) -> Value {
    json!({
        "requested_scope": cli_scope_name(inventory.requested_scope),
        "selected_scope": inventory.selected_scope.map(|scope| scope_name(scope)),
        "preferred_install_scope": inventory.preferred_install_scope.map(scope_name),
        "fallback_chain": inventory.fallback_chain.iter().map(|scope| scope_name(*scope)).collect::<Vec<_>>(),
        "selected_reason": inventory.selected_reason,
        "next_action": inventory.next_action.as_str(),
        "probe_chain": inventory.probe_chain.iter().map(probe_json).collect::<Vec<_>>(),
    })
}

fn requested_fallback_chain(requested_scope: cli::ServiceScope) -> Vec<ServiceScope> {
    match requested_scope {
        cli::ServiceScope::Auto => vec![ServiceScope::System, ServiceScope::User],
        cli::ServiceScope::User => vec![ServiceScope::User],
        cli::ServiceScope::System => vec![ServiceScope::System],
    }
}

fn select_scope(
    requested_scope: cli::ServiceScope,
    probe_chain: &[ServiceProbeSummary],
) -> Option<ServiceScope> {
    if matches!(requested_scope, cli::ServiceScope::Auto) {
        for scope in [ServiceScope::System, ServiceScope::User] {
            if probe_for_scope(probe_chain, scope).is_some_and(|probe| probe.exists) {
                return Some(scope);
            }
        }
        return Some(preferred_install_scope());
    }

    Some(match requested_scope {
        cli::ServiceScope::User => ServiceScope::User,
        cli::ServiceScope::System => ServiceScope::System,
        cli::ServiceScope::Auto => preferred_install_scope(),
    })
}

fn selected_reason(
    requested_scope: cli::ServiceScope,
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
        cli::ServiceScope::Auto => format!(
            "no persistent service was found; selected {:?} as the install target",
            scope
        ),
        cli::ServiceScope::User => "user scope was requested explicitly".to_string(),
        cli::ServiceScope::System => "system scope was requested explicitly".to_string(),
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

fn probe_for_scope<'a>(
    probe_chain: &'a [ServiceProbeSummary],
    scope: ServiceScope,
) -> Option<&'a ServiceProbeSummary> {
    probe_chain.iter().find(|probe| probe.scope == scope)
}

fn scope_name(scope: ServiceScope) -> &'static str {
    match scope {
        ServiceScope::User => "user",
        ServiceScope::System => "system",
    }
}

fn cli_scope_name(scope: cli::ServiceScope) -> &'static str {
    match scope {
        cli::ServiceScope::Auto => "auto",
        cli::ServiceScope::User => "user",
        cli::ServiceScope::System => "system",
    }
}

fn probe_json(probe: &ServiceProbeSummary) -> Value {
    json!({
        "scope": scope_name(probe.scope),
        "service_name": probe.service_name,
        "state": match probe.state {
            ServiceProbeState::Healthy => "healthy",
            ServiceProbeState::Present => "present",
            ServiceProbeState::Missing => "missing",
            ServiceProbeState::PermissionDenied => "permission_denied",
            ServiceProbeState::Unknown => "unknown",
        },
        "exists": probe.exists,
        "healthy": probe.healthy,
        "accessible": probe.accessible,
        "permission_denied": probe.permission_denied,
        "details": probe.details.clone(),
    })
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
            cli::ServiceScope::Auto,
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
            cli::ServiceScope::Auto,
            vec![
                probe(ServiceScope::System, false, false),
                probe(ServiceScope::User, false, false),
            ],
        );

        assert_eq!(inventory.selected_scope, Some(preferred_install_scope()));
        assert_eq!(inventory.next_action, ServiceNextAction::Install);
    }

    #[test]
    fn explicit_scope_is_kept_even_when_others_exist() {
        let inventory = resolve_service_inventory(
            cli::ServiceScope::System,
            vec![
                probe(ServiceScope::System, false, false),
                probe(ServiceScope::User, true, true),
            ],
        );

        assert_eq!(inventory.selected_scope, Some(ServiceScope::System));
        assert_eq!(inventory.next_action, ServiceNextAction::Install);
    }
}
