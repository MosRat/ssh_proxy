use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStagedUpdate {
    pub source: PathBuf,
    pub staged_path: PathBuf,
    pub hash: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonUpdateSwitchPlan {
    pub service_name: String,
    pub current_exe: PathBuf,
    pub backup_path: PathBuf,
    pub script_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonUpdatePlan {
    pub staged: DaemonStagedUpdate,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch: Option<DaemonUpdateSwitchPlan>,
}

impl DaemonUpdatePlan {
    pub fn new(staged: DaemonStagedUpdate) -> Self {
        Self {
            staged,
            switch: None,
        }
    }

    pub fn with_switch(mut self, switch: DaemonUpdateSwitchPlan) -> Self {
        self.switch = Some(switch);
        self
    }

    pub fn to_json(&self) -> Value {
        json!(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_plan_renders_public_paths() {
        let plan = DaemonUpdatePlan::new(DaemonStagedUpdate {
            source: PathBuf::from("candidate.exe"),
            staged_path: PathBuf::from("staged.exe"),
            hash: "abc123".to_string(),
            version: "0.1.2".to_string(),
        })
        .with_switch(DaemonUpdateSwitchPlan {
            service_name: "ssh_proxy".to_string(),
            current_exe: PathBuf::from("ssh_proxy.exe"),
            backup_path: PathBuf::from("backup.exe"),
            script_path: PathBuf::from("switch.ps1"),
        });

        let value = plan.to_json();

        assert_eq!(value["staged"]["version"], "0.1.2");
        assert_eq!(value["switch"]["service_name"], "ssh_proxy");
    }
}
