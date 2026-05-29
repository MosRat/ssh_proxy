use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::peer_lifecycle::{artifacts::PeerArtifact, executor::ServiceControlAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleOperation {
    Install,
    Ensure,
    Start,
    Stop,
    Status,
    Repair,
    Rollback,
}

impl LifecycleOperation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Ensure => "ensure",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Status => "status",
            Self::Repair => "repair",
            Self::Rollback => "rollback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PeerLifecyclePhase {
    Prepare,
    InspectDescriptor,
    DependencyCheck,
    StageBinary,
    WriteConfig,
    InstallService,
    StartService,
    HealthProbe,
    Record,
    Healthy,
    Repairing,
    Rollback,
    Failed,
}

impl PeerLifecyclePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::InspectDescriptor => "inspect_descriptor",
            Self::DependencyCheck => "dependency_check",
            Self::StageBinary => "stage_binary",
            Self::WriteConfig => "write_config",
            Self::InstallService => "install_service",
            Self::StartService => "start_service",
            Self::HealthProbe => "health_probe",
            Self::Record => "record",
            Self::Healthy => "healthy",
            Self::Repairing => "repairing",
            Self::Rollback => "rollback",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn progress(self) -> u8 {
        match self {
            Self::Prepare => 5,
            Self::InspectDescriptor => 15,
            Self::DependencyCheck => 25,
            Self::StageBinary => 35,
            Self::WriteConfig => 45,
            Self::InstallService => 60,
            Self::StartService => 72,
            Self::HealthProbe => 85,
            Self::Record => 95,
            Self::Healthy => 100,
            Self::Repairing => 50,
            Self::Rollback => 90,
            Self::Failed => 100,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleCommand {
    pub(crate) phase: PeerLifecyclePhase,
    pub(crate) command: String,
    pub(crate) stdin: Option<Vec<u8>>,
}

impl LifecycleCommand {
    pub(crate) fn new(phase: PeerLifecyclePhase, command: impl Into<String>) -> Self {
        Self {
            phase,
            command: command.into(),
            stdin: None,
        }
    }

    pub(crate) fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleCommandPlan {
    pub(crate) operation: LifecycleOperation,
    pub(crate) commands: Vec<LifecycleCommand>,
}

impl LifecycleCommandPlan {
    pub(crate) fn new(operation: LifecycleOperation) -> Self {
        Self {
            operation,
            commands: Vec::new(),
        }
    }

    pub(crate) fn push(mut self, command: LifecycleCommand) -> Self {
        self.commands.push(command);
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LifecycleAction {
    RunCommand {
        command: String,
        stdin: Option<Vec<u8>>,
    },
    StageBinary {
        source: String,
        target: String,
    },
    WriteArtifact {
        target: String,
        artifact: PeerArtifact,
        bytes: Vec<u8>,
    },
    ReadArtifact {
        target: String,
    },
    ProbeTcp {
        addr: SocketAddr,
    },
    ServiceControl {
        service_name: String,
        action: ServiceControlAction,
    },
    Noop,
}

impl LifecycleAction {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::RunCommand { .. } => "run_command",
            Self::StageBinary { .. } => "stage_binary",
            Self::WriteArtifact { .. } => "write_artifact",
            Self::ReadArtifact { .. } => "read_artifact",
            Self::ProbeTcp { .. } => "probe_tcp",
            Self::ServiceControl { .. } => "service_control",
            Self::Noop => "noop",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecycleStep {
    pub(crate) phase: PeerLifecyclePhase,
    pub(crate) action: LifecycleAction,
}

impl LifecycleStep {
    pub(crate) fn new(phase: PeerLifecyclePhase, action: LifecycleAction) -> Self {
        Self { phase, action }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LifecyclePlan {
    pub(crate) operation: LifecycleOperation,
    pub(crate) steps: Vec<LifecycleStep>,
}

impl LifecyclePlan {
    pub(crate) fn new(operation: LifecycleOperation) -> Self {
        Self {
            operation,
            steps: Vec::new(),
        }
    }

    pub(crate) fn push(mut self, step: LifecycleStep) -> Self {
        self.steps.push(step);
        self
    }
}

impl From<LifecycleCommandPlan> for LifecyclePlan {
    fn from(plan: LifecycleCommandPlan) -> Self {
        let steps = plan
            .commands
            .into_iter()
            .map(|command| {
                LifecycleStep::new(
                    command.phase,
                    LifecycleAction::RunCommand {
                        command: command.command,
                        stdin: command.stdin,
                    },
                )
            })
            .collect();
        Self {
            operation: plan.operation,
            steps,
        }
    }
}
