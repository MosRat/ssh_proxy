use anyhow::Result;

use crate::peer_lifecycle::{
    executor::{BoxExecutorFuture, LocalExecutor, PeerExecutor, ServiceControlAction},
    workflow::{
        LifecycleAction, LifecycleOperation, LifecyclePlan, LifecycleStep, PeerLifecyclePhase,
        PeerLifecycleWorkflowResult,
    },
};
use crate::ssh_client::ExecOutput;

use super::{plan, platform};

pub(super) async fn run_local_service_lifecycle(
    plan: &plan::ServicePlan,
    operation: LifecycleOperation,
) -> Result<PeerLifecycleWorkflowResult> {
    let executor = ServiceLifecycleExecutor::new(plan);
    let spec = plan.lifecycle_spec();
    let lifecycle = local_service_lifecycle_plan(plan, operation);
    let mut sink = crate::peer_lifecycle::workflow::VecLifecycleEventSink::default();
    crate::peer_lifecycle::workflow::run_lifecycle_plan(&executor, &spec, lifecycle, &mut sink)
        .await
}

#[cfg(test)]
pub(super) fn local_service_lifecycle_plan(
    plan: &plan::ServicePlan,
    operation: LifecycleOperation,
) -> LifecyclePlan {
    plan_local_service_lifecycle(plan, operation)
}

#[cfg(not(test))]
fn local_service_lifecycle_plan(
    plan: &plan::ServicePlan,
    operation: LifecycleOperation,
) -> LifecyclePlan {
    plan_local_service_lifecycle(plan, operation)
}

fn plan_local_service_lifecycle(
    plan: &plan::ServicePlan,
    operation: LifecycleOperation,
) -> LifecyclePlan {
    let mut lifecycle = LifecyclePlan::new(operation).push(LifecycleStep::new(
        PeerLifecyclePhase::DependencyCheck,
        LifecycleAction::Noop,
    ));
    match operation {
        LifecycleOperation::Install | LifecycleOperation::Ensure | LifecycleOperation::Repair => {
            lifecycle = lifecycle.push(LifecycleStep::new(
                PeerLifecyclePhase::StageBinary,
                LifecycleAction::StageBinary {
                    source: plan.source_exe.display().to_string(),
                    target: plan.exe.display().to_string(),
                },
            ));
            lifecycle.push(LifecycleStep::new(
                PeerLifecyclePhase::InstallService,
                LifecycleAction::ServiceControl {
                    service_name: plan::platform_service_name(plan.scope),
                    action: ServiceControlAction::Install,
                },
            ))
        }
        LifecycleOperation::Start => lifecycle.push(LifecycleStep::new(
            PeerLifecyclePhase::StartService,
            LifecycleAction::ServiceControl {
                service_name: plan::platform_service_name(plan.scope),
                action: ServiceControlAction::Start,
            },
        )),
        LifecycleOperation::Stop => lifecycle.push(LifecycleStep::new(
            PeerLifecyclePhase::Repairing,
            LifecycleAction::ServiceControl {
                service_name: plan::platform_service_name(plan.scope),
                action: ServiceControlAction::Stop,
            },
        )),
        LifecycleOperation::Status => lifecycle.push(LifecycleStep::new(
            PeerLifecyclePhase::HealthProbe,
            LifecycleAction::ServiceControl {
                service_name: plan::platform_service_name(plan.scope),
                action: ServiceControlAction::Status,
            },
        )),
        LifecycleOperation::Rollback => lifecycle.push(LifecycleStep::new(
            PeerLifecyclePhase::Rollback,
            LifecycleAction::ServiceControl {
                service_name: plan::platform_service_name(plan.scope),
                action: ServiceControlAction::Rollback,
            },
        )),
    }
}

struct ServiceLifecycleExecutor<'a> {
    plan: &'a plan::ServicePlan,
    local: LocalExecutor,
}

impl<'a> ServiceLifecycleExecutor<'a> {
    fn new(plan: &'a plan::ServicePlan) -> Self {
        Self {
            plan,
            local: LocalExecutor,
        }
    }
}

impl PeerExecutor for ServiceLifecycleExecutor<'_> {
    fn exec_capture<'a>(
        &'a self,
        command: String,
        stdin: Option<Vec<u8>>,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        self.local.exec_capture(command, stdin)
    }

    fn upload_bytes<'a>(&'a self, path: String, bytes: Vec<u8>) -> BoxExecutorFuture<'a, ()> {
        self.local.upload_bytes(path, bytes)
    }

    fn stage_binary<'a>(&'a self, _source: String, _target: String) -> BoxExecutorFuture<'a, ()> {
        Box::pin(async move {
            platform::platform_prepare_install(self.plan)?;
            self.plan.install_binary()
        })
    }

    fn service_control<'a>(
        &'a self,
        _service_name: String,
        action: ServiceControlAction,
    ) -> BoxExecutorFuture<'a, ExecOutput> {
        Box::pin(async move {
            let result = match action {
                ServiceControlAction::Install => platform::platform_install(self.plan),
                ServiceControlAction::Start => platform::platform_start(self.plan),
                ServiceControlAction::Stop => platform::platform_stop(self.plan),
                ServiceControlAction::Status => Ok(()),
                ServiceControlAction::Rollback => platform::platform_uninstall(self.plan),
            };
            match result {
                Ok(()) => Ok(ExecOutput {
                    exit_status: 0,
                    stdout: match action {
                        ServiceControlAction::Status => {
                            platform::platform_status_summary(self.plan).to_string()
                        }
                        _ => String::new(),
                    },
                    stderr: String::new(),
                }),
                Err(err) => Ok(ExecOutput {
                    exit_status: 1,
                    stdout: String::new(),
                    stderr: format!("{err:#}"),
                }),
            }
        })
    }
}
