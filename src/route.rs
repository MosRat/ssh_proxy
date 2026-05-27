mod builders;
mod policy;
mod preflight;
mod request;
mod response;
mod runner;
mod selection;
mod transport;
pub(crate) use builders::{
    install_args_from_route, node_forward_from_route, node_reverse_from_route,
    remote_direct_host_args, route_id,
};
pub(crate) use preflight::{add_local_transport_probe_results, apply_local_forward_fallback};
pub(crate) use request::{
    reverse_route_start_request, route_intent_request, route_start_request,
    route_start_request_with_reason,
};
pub(crate) use response::{
    local_uses_remote_plan, remote_uses_local_direct_plan, remote_uses_local_reverse_link_plan,
};
pub(crate) use runner::explain_plan;
pub use runner::run;
pub(crate) use selection::{
    RemoteUseDecision, RemoteUsePlan, local_peer_addr, remote_use_decision, route_deploy_mode,
    transport_selection_policy,
};
pub(crate) use transport::{
    direct_transport_policy, direct_transport_policy_reason, parse_remote_os,
    parse_remote_transport, remote_transport_name, ssh_data_plane_reason, ssh_mode_name,
    ssh_mode_reason, tls_peer_auth_mode,
};

#[cfg(test)]
mod tests;
