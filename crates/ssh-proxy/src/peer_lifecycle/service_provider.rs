mod kind;
mod plans;
mod selection;
mod status;

pub(crate) use kind::ServiceProviderKind;
#[allow(unused_imports)]
pub(crate) use plans::{RemoteServiceInstallPlan, remote_service_install_plan};
#[allow(unused_imports)]
pub(crate) use selection::{provider_for_platform, provider_for_remote_report};
#[allow(unused_imports)]
pub(crate) use ssh_proxy_lifecycle::service_provider::{PeerServiceProvider, ServiceProviderPlan};
#[allow(unused_imports)]
pub(crate) use status::{ProviderStatus, ProviderStatusState};

#[cfg(test)]
mod tests;
