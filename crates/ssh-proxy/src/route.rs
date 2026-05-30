mod builders;
mod policy;
mod preflight;
mod request;
mod response;
mod runner;
mod runtime_decision;
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
pub(crate) use runtime_decision::RouteRuntimeDecision;
#[cfg(test)]
pub(crate) use selection::local_peer_addr;
pub(crate) use selection::{RemoteUsePlan, remote_use_decision};

#[cfg(test)]
mod tests;
