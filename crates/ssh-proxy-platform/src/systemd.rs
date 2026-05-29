use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ssh_proxy_core::external::ExternalActionClass;

use crate::{ExecutionBackend, NativeProviderOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemdScope {
    User,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemdOperation {
    Reload,
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
    Status,
    SetUserLinger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemdDbusPlan {
    pub scope: SystemdScope,
    pub operation: SystemdOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_linger_uid: Option<u32>,
}

impl SystemdDbusPlan {
    pub fn reload(scope: SystemdScope) -> Self {
        Self {
            scope,
            operation: SystemdOperation::Reload,
            unit: None,
            mode: None,
            user_linger_uid: None,
        }
    }

    pub fn unit(scope: SystemdScope, operation: SystemdOperation, unit: impl Into<String>) -> Self {
        Self {
            scope,
            operation,
            unit: Some(unit.into()),
            mode: Some("replace".to_string()),
            user_linger_uid: None,
        }
    }

    pub fn set_user_linger(uid: u32) -> Self {
        Self {
            scope: SystemdScope::System,
            operation: SystemdOperation::SetUserLinger,
            unit: None,
            mode: None,
            user_linger_uid: Some(uid),
        }
    }

    pub fn method_name(&self) -> &'static str {
        match self.operation {
            SystemdOperation::Reload => "Reload",
            SystemdOperation::Start => "StartUnit",
            SystemdOperation::Stop => "StopUnit",
            SystemdOperation::Restart => "RestartUnit",
            SystemdOperation::Enable => "EnableUnitFiles",
            SystemdOperation::Disable => "DisableUnitFiles",
            SystemdOperation::Status => "GetUnit",
            SystemdOperation::SetUserLinger => "SetUserLinger",
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "execution_backend": ExecutionBackend::Dbus.as_str(),
            "bus": if self.operation == SystemdOperation::SetUserLinger {
                "system"
            } else {
                match self.scope {
                    SystemdScope::User => "session",
                    SystemdScope::System => "system",
                }
            },
            "destination": if self.operation == SystemdOperation::SetUserLinger {
                "org.freedesktop.login1"
            } else {
                "org.freedesktop.systemd1"
            },
            "interface": if self.operation == SystemdOperation::SetUserLinger {
                "org.freedesktop.login1.Manager"
            } else {
                "org.freedesktop.systemd1.Manager"
            },
            "method": self.method_name(),
            "unit": self.unit,
            "scope": self.scope,
            "operation": self.operation,
            "user_linger_uid": self.user_linger_uid,
        })
    }
}

#[cfg(unix)]
pub fn run_systemd_plan(plan: &SystemdDbusPlan) -> anyhow::Result<NativeProviderOutcome> {
    use anyhow::{Context, bail};
    use zbus::blocking::{Connection, Proxy};

    let connection = if plan.operation == SystemdOperation::SetUserLinger {
        Connection::system().context("failed to connect to system D-Bus for logind")?
    } else {
        match plan.scope {
            SystemdScope::User => {
                Connection::session().context("failed to connect to user/session D-Bus")?
            }
            SystemdScope::System => {
                Connection::system().context("failed to connect to system D-Bus")?
            }
        }
    };

    if plan.operation == SystemdOperation::SetUserLinger {
        let uid = plan
            .user_linger_uid
            .ok_or_else(|| anyhow::anyhow!("SetUserLinger requires a uid"))?;
        let proxy = Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .context("failed to create logind D-Bus proxy")?;
        proxy
            .call_method("SetUserLinger", &(uid, true, false))
            .context("failed to call logind SetUserLinger")?;
        return Ok(native_outcome(plan, true).with_status("linger_enabled"));
    }

    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .context("failed to create systemd D-Bus proxy")?;

    match plan.operation {
        SystemdOperation::Reload => {
            proxy
                .call_method("Reload", &())
                .context("failed to reload systemd manager through D-Bus")?;
        }
        SystemdOperation::Start | SystemdOperation::Stop | SystemdOperation::Restart => {
            let unit = plan.unit.as_deref().context("unit operation requires unit")?;
            let mode = plan.mode.as_deref().unwrap_or("replace");
            proxy
                .call_method(plan.method_name(), &(unit, mode))
                .with_context(|| format!("failed to call {} for {unit}", plan.method_name()))?;
        }
        SystemdOperation::Enable => {
            let unit = plan.unit.as_deref().context("enable requires unit")?;
            proxy
                .call_method("EnableUnitFiles", &(vec![unit], false, true))
                .with_context(|| format!("failed to enable {unit} through D-Bus"))?;
        }
        SystemdOperation::Disable => {
            let unit = plan.unit.as_deref().context("disable requires unit")?;
            proxy
                .call_method("DisableUnitFiles", &(vec![unit], false))
                .with_context(|| format!("failed to disable {unit} through D-Bus"))?;
        }
        SystemdOperation::Status => {
            let unit = plan.unit.as_deref().context("status requires unit")?;
            proxy
                .call_method("GetUnit", &(unit))
                .with_context(|| format!("failed to query {unit} through D-Bus"))?;
        }
        SystemdOperation::SetUserLinger => bail!("SetUserLinger is handled through logind"),
    }

    Ok(native_outcome(plan, true))
}

#[cfg(not(unix))]
pub fn run_systemd_plan(plan: &SystemdDbusPlan) -> anyhow::Result<NativeProviderOutcome> {
    Ok(native_outcome(plan, false)
        .with_status("unsupported")
        .with_message("systemd D-Bus is only available on Unix targets"))
}

fn native_outcome(plan: &SystemdDbusPlan, ok: bool) -> NativeProviderOutcome {
    NativeProviderOutcome::new(
        ok,
        ExecutionBackend::Dbus,
        ExternalActionClass::RequiredProvider,
        format!("run {} through systemd/logind D-Bus", plan.method_name()),
    )
    .with_details(plan.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_plan_renders_dbus_request_shape() {
        let plan = SystemdDbusPlan::unit(
            SystemdScope::User,
            SystemdOperation::Start,
            "ssh_proxy.service",
        );
        let value = plan.to_json();

        assert_eq!(value["execution_backend"], "dbus");
        assert_eq!(value["bus"], "session");
        assert_eq!(value["destination"], "org.freedesktop.systemd1");
        assert_eq!(value["method"], "StartUnit");
        assert_eq!(value["unit"], "ssh_proxy.service");
    }

    #[test]
    fn logind_linger_plan_uses_system_bus() {
        let plan = SystemdDbusPlan::set_user_linger(1000);
        let value = plan.to_json();

        assert_eq!(value["bus"], "system");
        assert_eq!(value["destination"], "org.freedesktop.login1");
        assert_eq!(value["method"], "SetUserLinger");
        assert_eq!(value["user_linger_uid"], 1000);
    }
}
