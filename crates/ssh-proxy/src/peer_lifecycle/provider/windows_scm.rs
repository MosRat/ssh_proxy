#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowsScmPlan {
    pub(crate) service_name: String,
    pub(crate) command: String,
    pub(crate) uses_elevated_worker: bool,
    pub(crate) versioned_binary: bool,
}

impl WindowsScmPlan {
    pub(crate) fn new(service_name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            command: command.into(),
            uses_elevated_worker: true,
            versioned_binary: true,
        }
    }

    pub(crate) fn install_hint(&self) -> String {
        format!(
            "windows-service install-worker service={} command={}",
            self.service_name, self.command
        )
    }

    pub(crate) fn status_hint(&self) -> String {
        format!("sc.exe query {}", self.service_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_scm_plan_preserves_installer_contract() {
        let plan = WindowsScmPlan::new("ssh_proxy", "ssh_proxy daemon serve");

        assert!(plan.uses_elevated_worker);
        assert!(plan.versioned_binary);
        assert!(plan.install_hint().contains("install-worker"));
        assert_eq!(plan.status_hint(), "sc.exe query ssh_proxy");
    }
}
