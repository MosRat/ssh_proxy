use serde_json::{Value, json};

use crate::{cli, install_report, peer_lifecycle, repair};

use super::{
    labels::{cli_service_scope_name, service_scope_name},
    plan::{self, ServicePlan},
};

pub(super) fn install_success_report(plan: &ServicePlan) -> Value {
    let mut report = install_report::install_report_for_current_process();
    if report.get("state").and_then(Value::as_str) == Some("unknown") {
        report = json!({
            "ok": true,
            "kind": "daemon_install",
            "daemon_api": "v0.3",
            "state": "healthy",
            "phase": "healthy",
            "install_id": serde_json::Value::Null,
            "log_path": serde_json::Value::Null,
            "blocker": serde_json::Value::Null,
            "repair_action": serde_json::Value::Null,
            "health_check": {
                "state": "passed"
            },
        });
    }
    enrich_install_report(report, plan)
}

pub(super) fn install_failure_report(plan: &ServicePlan, err: &anyhow::Error) -> Value {
    let mut report = install_report::install_report_for_current_process();
    if report.get("state").and_then(Value::as_str) == Some("unknown") {
        let error = err.to_string();
        let blocker = if is_cancelled_install_error(&error) {
            "cancelled_by_user"
        } else if requires_elevation(plan, &error) {
            "requires_elevation"
        } else {
            "daemon_install_failed"
        };
        let state = if blocker == "cancelled_by_user" {
            "cancelled"
        } else {
            "failed"
        };
        report = json!({
            "ok": false,
            "kind": "daemon_install",
            "daemon_api": "v0.3",
            "state": state,
            "phase": blocker,
            "install_id": serde_json::Value::Null,
            "log_path": install_report::install_log_path_for_pid(std::process::id()),
            "message": error,
            "blocker": blocker,
            "repair_action": repair::action_value_for_blocker(blocker),
            "health_check": {
                "state": if blocker == "cancelled_by_user" { "skipped" } else { "failed" }
            },
        });
    }
    enrich_install_report(report, plan)
}

pub(super) fn service_operation(
    plan: &ServicePlan,
) -> peer_lifecycle::workflow::LifecycleOperation {
    match plan.command {
        cli::ServiceCommand::Install => peer_lifecycle::workflow::LifecycleOperation::Install,
        cli::ServiceCommand::Ensure => peer_lifecycle::workflow::LifecycleOperation::Ensure,
        cli::ServiceCommand::Start => peer_lifecycle::workflow::LifecycleOperation::Start,
        cli::ServiceCommand::Stop => peer_lifecycle::workflow::LifecycleOperation::Stop,
        cli::ServiceCommand::Status => peer_lifecycle::workflow::LifecycleOperation::Status,
        cli::ServiceCommand::Uninstall => peer_lifecycle::workflow::LifecycleOperation::Rollback,
        cli::ServiceCommand::Print => peer_lifecycle::workflow::LifecycleOperation::Status,
    }
}

pub(super) fn requires_elevation(plan: &ServicePlan, error: &str) -> bool {
    matches!(plan.scope, plan::ServiceScope::System)
        && !plan::is_admin()
        && (error.contains("administrator")
            || error.contains("root")
            || is_permission_denied_error(error))
}

pub(super) fn is_permission_denied_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("privilege")
        || lower.contains("elevation")
}

fn enrich_install_report(mut report: Value, plan: &ServicePlan) -> Value {
    let lifecycle = local_service_lifecycle_report(plan, &report);
    let service_manager = plan.lifecycle_spec().provider.manager_name().to_string();
    if let Some(object) = report.as_object_mut() {
        object.insert("installed_binary".to_string(), json!(plan.exe));
        object.insert("control".to_string(), json!(plan.endpoint));
        object.insert("scope".to_string(), json!(service_scope_name(plan.scope)));
        object.insert("service_manager".to_string(), json!(service_manager));
        object.insert("lifecycle".to_string(), lifecycle);
        object.insert(
            "requested_scope".to_string(),
            json!(cli_service_scope_name(plan.requested_scope)),
        );
    }
    report
}

fn local_service_lifecycle_report(plan: &ServicePlan, report: &Value) -> Value {
    let spec = plan.lifecycle_spec();
    let state = report
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let phase = match state {
        "healthy" => peer_lifecycle::workflow::PeerLifecyclePhase::Healthy,
        "cancelled" => peer_lifecycle::workflow::PeerLifecyclePhase::Rollback,
        "failed" => peer_lifecycle::workflow::PeerLifecyclePhase::Failed,
        _ => peer_lifecycle::workflow::PeerLifecyclePhase::InstallService,
    };
    let mut lifecycle =
        peer_lifecycle::workflow::phase_report_for_operation(&spec, service_operation(plan), phase);
    lifecycle.blocker = report
        .get("blocker")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    lifecycle.last_error = report
        .get("message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    lifecycle.to_redacted_value()
}

fn is_cancelled_install_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("cancelled_by_user")
        || lower.contains("code 1223")
        || lower.contains("operation was canceled")
        || lower.contains("operation was cancelled")
        || lower.contains("the operation was canceled by the user")
        || lower.contains("0xc000013a")
}
