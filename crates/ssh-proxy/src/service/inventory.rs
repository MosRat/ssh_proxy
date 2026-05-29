use serde_json::Value;
use ssh_proxy_service::{RequestedServiceScope, ServiceScope};
pub(crate) use ssh_proxy_service::{
    ServiceInventory, ServiceNextAction, ServiceProbeState, ServiceProbeSummary,
};

use crate::cli;

use super::plan::preferred_install_scope;

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
    ssh_proxy_service::resolve_service_inventory(
        requested_scope_from_cli(requested_scope),
        preferred_install_scope(),
        probe_chain,
    )
}

pub(crate) fn inventory_json(inventory: &ServiceInventory) -> Value {
    inventory.to_json()
}

fn requested_scope_from_cli(scope: cli::ServiceScope) -> RequestedServiceScope {
    match scope {
        cli::ServiceScope::Auto => RequestedServiceScope::Auto,
        cli::ServiceScope::User => RequestedServiceScope::User,
        cli::ServiceScope::System => RequestedServiceScope::System,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

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
