use serde_json::Value;
use ssh_proxy_service::ServiceBrokerReportInput;

use super::plan::ServicePlan;

pub(super) fn broker_json(
    plan: &ServicePlan,
    daemon_reachable: bool,
    platform_ok: bool,
    requires_elevation: bool,
) -> Value {
    ssh_proxy_service::service_broker_report(ServiceBrokerReportInput {
        scope: plan.scope,
        endpoint: &plan.endpoint,
        default_endpoint: &crate::control_socket::default_endpoint_string(),
        system_endpoint: &system_endpoint(),
        daemon_reachable,
        platform_ok,
        requires_elevation,
        current_user_is_admin: super::plan::is_admin(),
        probe_chain: &plan.resolution.probe_chain,
    })
}

fn system_endpoint() -> String {
    #[cfg(windows)]
    {
        "npipe://ssh_proxy/system/control".to_string()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "unix:///run/ssh_proxy/control.sock".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "unix:///var/run/ssh_proxy/control.sock".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cli, config};
    use serde_json::json;

    fn plan(scope: cli::ServiceScope) -> ServicePlan {
        ServicePlan::new(
            cli::ServiceArgs {
                scope,
                control: Some("tcp://127.0.0.1:1".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                no_transport: false,
                token: Some("secret".to_string()),
                tls_transport: None,
                quic_transport: None,
                tls_cert: None,
                tls_key: None,
                tls_client_ca: None,
                report_to: vec![],
                install_dir: None,
                no_copy: true,
                json: true,
                elevate: false,
                command: cli::ServiceCommand::Status,
            },
            config::AppConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn broker_json_prefers_reachable_current_control() {
        let value = broker_json(&plan(cli::ServiceScope::User), true, true, false);

        assert_eq!(value["next_action"], "reuse_broker");
        assert!(value["selected"]["reachable"].as_bool().unwrap());
        assert!(
            value["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("route_intent"))
        );
    }

    #[test]
    fn broker_json_never_grants_arbitrary_shell() {
        let value = broker_json(&plan(cli::ServiceScope::System), false, false, true);

        assert_eq!(value["policy"]["arbitrary_shell"], false);
        assert_eq!(value["permission_boundary"]["arbitrary_shell"], false);
        assert!(value["next_action"].as_str().unwrap().contains("elevated"));
    }
}
