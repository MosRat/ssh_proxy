use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::{cli, config};

mod broker;
mod executor_adapter;
mod health;
mod inventory;
mod labels;
mod peer_health;
mod plan;
mod platform;
mod report;
mod status;

#[cfg(test)]
use executor_adapter::local_service_lifecycle_plan;
use executor_adapter::run_local_service_lifecycle;
use inventory::ServiceNextAction;
use labels::{platform_service_name, service_scope_name};
use plan::ServicePlan;
use report::{
    install_failure_report, install_success_report, is_permission_denied_error, requires_elevation,
    service_operation,
};
use status::{service_status_summary, status_service};

pub async fn run(args: cli::ServiceArgs, config: config::AppConfig) -> Result<()> {
    let json = args.json;
    let plan = ServicePlan::new(args, config)?;
    match plan.command {
        cli::ServiceCommand::Print => print_service(&plan),
        cli::ServiceCommand::Ensure => ensure_service(&plan, json).await,
        cli::ServiceCommand::Install => match install_service(&plan).await {
            Ok(()) => {
                if json {
                    println!("{}", serde_json::to_string(&install_success_report(&plan))?);
                }
                Ok(())
            }
            Err(err) => {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string(&install_failure_report(&plan, &err))?
                    );
                }
                Err(err)
            }
        },
        cli::ServiceCommand::Uninstall => {
            run_local_service_lifecycle(&plan, service_operation(&plan))
                .await
                .map(|_| ())
        }
        cli::ServiceCommand::Start => run_local_service_lifecycle(&plan, service_operation(&plan))
            .await
            .map(|_| ()),
        cli::ServiceCommand::Stop => run_local_service_lifecycle(&plan, service_operation(&plan))
            .await
            .map(|_| ()),
        cli::ServiceCommand::Status => status_service(&plan, json).await,
    }
}

async fn ensure_service(plan: &ServicePlan, json_output: bool) -> Result<()> {
    let before = service_status_summary(plan).await?;
    if before["ok"].as_bool().unwrap_or(false) {
        if json_output {
            println!("{}", serde_json::to_string(&before)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&before)?);
        }
        return Ok(());
    }

    let outcome = match install_or_repair_service(plan).await {
        Ok(()) => service_status_summary(plan).await?,
        Err(err) => {
            let mut value = service_status_summary(plan).await?;
            if let Some(object) = value.as_object_mut() {
                object.insert("ok".to_string(), Value::Bool(false));
                object.insert("ensure_error".to_string(), Value::String(err.to_string()));
                object.insert(
                    "next_action".to_string(),
                    Value::String(if is_permission_denied_error(&err.to_string()) {
                        "session_daemon".to_string()
                    } else if requires_elevation(plan, &err.to_string()) {
                        "install_system_elevated".to_string()
                    } else {
                        "session_daemon".to_string()
                    }),
                );
                object.insert(
                    "requires_elevation".to_string(),
                    Value::Bool(requires_elevation(plan, &err.to_string())),
                );
            }
            value
        }
    };

    if json_output {
        println!("{}", serde_json::to_string(&outcome)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    }
    Ok(())
}

fn print_service(plan: &ServicePlan) -> Result<()> {
    println!("ssh_proxy {}", env!("CARGO_PKG_VERSION"));
    println!("config: {}", plan.config_path.display());
    println!("scope: {:?}", plan.scope);
    if plan.copy_exe {
        println!("installed binary: {}", plan.exe.display());
    }
    println!("daemon command:");
    println!("  {}", plan.daemon_command());
    if let Some(transport) = plan.transport {
        println!("transport: tcp://{transport}");
    } else {
        println!("transport: disabled");
    }
    if let Some(transport) = plan.tls_transport {
        println!("tls transport: tls://{transport}");
    }
    if let Some(transport) = plan.quic_transport {
        println!("quic transport: quic://{transport}");
    }
    println!();
    platform::platform_print(plan)
}

async fn install_service(plan: &ServicePlan) -> Result<()> {
    install_or_repair_service(plan).await
}

async fn install_or_repair_service(plan: &ServicePlan) -> Result<()> {
    let action = plan.resolution.next_action;
    if matches!(
        action,
        ServiceNextAction::Reuse | ServiceNextAction::StartOrRepair | ServiceNextAction::Install
    ) && platform::platform_install_requires_elevation(plan)
    {
        return platform::platform_install(plan);
    }
    let original_config = if plan.config_to_save.is_some() {
        match fs::read(&plan.config_path) {
            Ok(bytes) => Some(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(None),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed to snapshot existing config {}",
                        plan.config_path.display()
                    )
                });
            }
        }
    } else {
        None
    };

    match action {
        ServiceNextAction::Reuse if !matches!(plan.command, cli::ServiceCommand::Install) => {
            println!(
                "selected existing {} service at {}; no install required",
                service_scope_name(plan.scope),
                platform_service_name(plan.scope)
            );
            return Ok(());
        }
        ServiceNextAction::Reuse
        | ServiceNextAction::StartOrRepair
        | ServiceNextAction::Install => {
            let install_result = async {
                if let Some(config) = &plan.config_to_save {
                    config.save_default()?;
                    println!("saved daemon defaults to {}", plan.config_path.display());
                }
                run_local_service_lifecycle(plan, service_operation(plan))
                    .await
                    .map(|_| ())
            }
            .await;
            if let Err(err) = install_result {
                if let Some(snapshot) = original_config {
                    restore_config_snapshot(&plan.config_path, snapshot)?;
                    eprintln!(
                        "rolled back daemon defaults in {} after service install failure",
                        plan.config_path.display()
                    );
                }
                return Err(err);
            }
        }
        ServiceNextAction::Unavailable => {
            if let Some(snapshot) = original_config {
                restore_config_snapshot(&plan.config_path, snapshot)?;
            }
            return Err(anyhow::anyhow!(
                "no persistent service scope could be selected; no install target available"
            ));
        }
    }
    Ok(())
}

fn restore_config_snapshot(path: &Path, snapshot: Option<Vec<u8>>) -> Result<()> {
    match snapshot {
        Some(bytes) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(path, bytes)
                .with_context(|| format!("failed to restore {}", path.display()))?;
        }
        None => match fs::remove_file(path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("failed to remove {}", path.display()));
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_lifecycle;
    use crate::peer_lifecycle::workflow::PeerLifecyclePhase;

    fn status_plan() -> ServicePlan {
        ServicePlan::new(
            cli::ServiceArgs {
                scope: cli::ServiceScope::User,
                control: Some("tcp://127.0.0.1:1".to_string()),
                transport: Some("127.0.0.1:19080".parse().unwrap()),
                no_transport: false,
                token: Some("secret".to_string()),
                tls_transport: None,
                quic_transport: Some("127.0.0.1:19083".parse().unwrap()),
                tls_cert: None,
                tls_key: None,
                tls_client_ca: None,
                report_to: vec!["tcp://127.0.0.1:19091".to_string()],
                install_dir: None,
                no_copy: true,
                json: false,
                elevate: false,
                command: cli::ServiceCommand::Status,
            },
            config::AppConfig::default(),
        )
        .unwrap()
    }

    fn service_args(command: cli::ServiceCommand) -> cli::ServiceArgs {
        cli::ServiceArgs {
            scope: cli::ServiceScope::User,
            control: Some("tcp://127.0.0.1:1".to_string()),
            transport: Some("127.0.0.1:19080".parse().unwrap()),
            no_transport: false,
            token: None,
            tls_transport: None,
            quic_transport: None,
            tls_cert: None,
            tls_key: None,
            tls_client_ca: None,
            report_to: Vec::new(),
            install_dir: None,
            no_copy: true,
            json: false,
            elevate: false,
            command,
        }
    }

    #[tokio::test]
    async fn service_status_summary_is_redacted_and_structured() {
        let plan = status_plan();
        let summary = status::service_status_summary(&plan).await.unwrap();

        assert_eq!(summary["kind"], "service_status");
        assert_eq!(summary["scope"], "user");
        assert_eq!(summary["auth"]["token"], true);
        assert_eq!(summary["transport"]["plain_tcp"], "127.0.0.1:19080");
        assert_eq!(summary["transport"]["quic"], "127.0.0.1:19083");
        assert_eq!(summary["health"]["listeners"]["control"]["ok"], true);
        assert_eq!(summary["health"]["route_store"]["ok"], true);
        assert_eq!(summary["health"]["listeners"]["quic"]["configured"], true);
        assert!(summary["state"].is_string());
        assert_eq!(
            summary["manager"]["session_daemon_fallback"]["supported"],
            true
        );
        assert!(summary["manager"]["next_action"].is_string());
        assert!(summary["platform"]["status"]["ok"].is_boolean());
        assert!(!summary.to_string().contains("secret"));
        assert!(summary["daemon"]["reachable"].is_boolean());
    }

    #[test]
    fn service_state_names_cover_core_cases() {
        assert_eq!(
            status::service_state_name(true, true),
            "running_with_persistent_manager"
        );
        assert_eq!(
            status::service_state_name(true, false),
            "running_without_persistent_manager"
        );
        assert_eq!(
            status::service_state_name(false, true),
            "persistent_manager_registered_but_daemon_unreachable"
        );
        assert_eq!(status::service_state_name(false, false), "unavailable");
        assert_eq!(
            status::service_next_action(true, false),
            "reuse_default_daemon"
        );
        assert_eq!(
            status::service_next_action(false, true),
            "start_or_repair_persistent_service"
        );
        assert_eq!(
            status::service_next_action(false, false),
            "install_persistent_service_or_start_session_daemon"
        );
    }

    #[test]
    fn explicit_install_token_is_saved_to_materialized_config() {
        let mut args = service_args(cli::ServiceCommand::Install);
        args.token = Some("install-token".to_string());

        let plan = ServicePlan::new(args, config::AppConfig::default()).unwrap();
        let saved = plan
            .config_to_save
            .as_ref()
            .expect("config should be saved");

        assert_eq!(plan.token.as_deref(), Some("install-token"));
        assert_eq!(saved.daemon.token.as_deref(), Some("install-token"));
        assert_eq!(
            saved
                .daemon
                .token_metadata
                .as_ref()
                .expect("token metadata")
                .scope,
            "daemon-control-transport"
        );
    }

    #[test]
    fn local_service_lifecycle_plan_models_install_and_start() {
        let install_plan = ServicePlan::new(
            service_args(cli::ServiceCommand::Install),
            config::AppConfig::default(),
        )
        .unwrap();
        let install_lifecycle = local_service_lifecycle_plan(
            &install_plan,
            peer_lifecycle::workflow::LifecycleOperation::Install,
        );

        assert_eq!(install_lifecycle.operation.as_str(), "install");
        assert_eq!(
            install_lifecycle.steps[0].phase,
            PeerLifecyclePhase::DependencyCheck
        );
        assert_eq!(
            install_lifecycle.steps[1].phase,
            PeerLifecyclePhase::StageBinary
        );
        assert_eq!(
            install_lifecycle.steps[2].phase,
            PeerLifecyclePhase::InstallService
        );

        let start_plan = ServicePlan::new(
            service_args(cli::ServiceCommand::Start),
            config::AppConfig::default(),
        )
        .unwrap();
        let start_lifecycle = local_service_lifecycle_plan(
            &start_plan,
            peer_lifecycle::workflow::LifecycleOperation::Start,
        );

        assert_eq!(start_lifecycle.operation.as_str(), "start");
        assert_eq!(
            start_lifecycle.steps[1].phase,
            PeerLifecyclePhase::StartService
        );
    }

    #[test]
    fn route_store_health_reports_invalid_json() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-invalid-route-store-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "{not-json").unwrap();

        let health = health::route_store_health(&path);

        assert_eq!(health["ok"], false);
        assert_eq!(health["exists"], true);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn route_store_health_reports_duplicate_ids() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-duplicate-route-store-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"version":1,"routes":[{"id":"same"},{"id":"same"}]}"#,
        )
        .unwrap();

        let health = health::route_store_health(&path);

        assert_eq!(health["ok"], false);
        assert_eq!(health["duplicate_ids"][0], "same");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn config_file_health_reports_future_schema() {
        let path = std::env::temp_dir().join(format!(
            "ssh_proxy-future-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "schema_version = 999\n").unwrap();

        let (health, _) = health::config_file_health_with_snapshot(&path);

        assert_eq!(health["ok"], false);
        assert!(
            health["error"]
                .as_str()
                .expect("error")
                .contains("newer than this binary supports")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unspecified_listener_probe_uses_loopback() {
        let addr = "0.0.0.0:19080".parse().unwrap();
        let probe = health::local_probe_addr(addr);

        assert_eq!(probe.to_string(), "127.0.0.1:19080");
    }

    #[tokio::test]
    async fn peer_registry_health_is_sorted_and_redacted() {
        let path =
            std::env::temp_dir().join(format!("ssh_proxy-peer-health-{}.toml", std::process::id()));
        let mut config = config::AppConfig::default();
        config.peers.insert(
            "zeta".to_string(),
            config::PeerRecord {
                node_id: Some("node-z".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                control_api_version: Some(crate::node_daemon::control_api_version()),
                peer_protocol_version: Some(crate::node_daemon::peer_protocol_version()),
                features: crate::node_daemon::peer_protocol_features(),
                control_endpoint: Some("tcp://127.0.0.1:1".to_string()),
                token: Some("peer-secret".to_string()),
                ..Default::default()
            },
        );
        config.peers.insert(
            "alpha".to_string(),
            config::PeerRecord {
                node_name: Some("node-a".to_string()),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                control_api_version: Some(crate::node_daemon::control_api_version()),
                peer_protocol_version: Some(crate::node_daemon::peer_protocol_version()),
                features: crate::node_daemon::peer_protocol_features(),
                control_endpoint: Some("tcp://127.0.0.1:1".to_string()),
                transport_protocols: vec!["plain-tcp".to_string()],
                ..Default::default()
            },
        );
        std::fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let (_, config) = health::config_file_health_with_snapshot(&path);
        let health = health::peer_registry_health(config.as_ref()).await;

        assert_eq!(health["ok"], true);
        assert_eq!(health["count"], 2);
        assert_eq!(health["peers"][0]["alias"], "alpha");
        assert_eq!(health["peers"][0]["compatibility"]["ok"], true);
        assert_eq!(health["peers"][1]["alias"], "zeta");
        assert_eq!(health["peers"][1]["auth"]["token"], true);
        assert!(!health.to_string().contains("peer-secret"));
        let _ = std::fs::remove_file(path);
    }
}
