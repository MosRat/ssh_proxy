use std::{collections::HashMap, net::SocketAddr};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use ssh_proxy_route::{
    RouteConflictDecision, RouteConflictInput, RouteConflictRoute, decide_route_conflict,
    route_matches,
};

use super::{RouteSpec, RouteTask};

pub(super) fn ensure_new_route_can_start(
    routes: &HashMap<String, RouteTask>,
    direction: &str,
    listen: SocketAddr,
    peer: Option<&str>,
    spec: &RouteSpec,
) -> Result<()> {
    let candidate = route_conflict_input(None, direction, listen, peer, spec)?;
    match decide_route_conflict(&route_conflict_routes(routes)?, &candidate) {
        RouteConflictDecision::Available => {}
        RouteConflictDecision::ListenerReserved { route_id } => {
            bail!("route {route_id:?} already owns {direction} listener {listen}");
        }
        RouteConflictDecision::ReuseExisting { .. }
        | RouteConflictDecision::DifferentSpec { .. } => {}
    }
    if matches!(spec, RouteSpec::Forward { .. }) {
        ensure_port_available(listen)?;
    }
    Ok(())
}

pub(super) fn route_task_matches(
    task: &RouteTask,
    direction: &str,
    listen: SocketAddr,
    peer: Option<&str>,
    spec: &RouteSpec,
) -> bool {
    let Ok(route) = route_conflict_route("_candidate", task) else {
        return false;
    };
    let Ok(candidate) = route_conflict_input(None, direction, listen, peer, spec) else {
        return false;
    };
    route_matches(&route, &candidate)
}

#[cfg(test)]
pub(super) fn route_specs_match(left: &RouteSpec, right: &RouteSpec) -> bool {
    match (route_spec_value(left), route_spec_value(right)) {
        (Ok(left), Ok(right)) => ssh_proxy_route::route_specs_match_values(&left, &right),
        _ => false,
    }
}

fn ensure_port_available(addr: SocketAddr) -> Result<()> {
    let listener = std::net::TcpListener::bind(addr)
        .with_context(|| format!("listen address {addr} is already in use or unavailable"))?;
    drop(listener);
    Ok(())
}

fn route_conflict_routes(routes: &HashMap<String, RouteTask>) -> Result<Vec<RouteConflictRoute>> {
    routes
        .iter()
        .map(|(id, task)| route_conflict_route(id, task))
        .collect()
}

fn route_conflict_route(id: &str, task: &RouteTask) -> Result<RouteConflictRoute> {
    Ok(RouteConflictRoute {
        id: id.to_string(),
        direction: task.direction.clone(),
        listen: task.listen.map(|addr| addr.to_string()),
        peer: task.peer.clone(),
        spec: route_spec_value(&task.spec)?,
    })
}

fn route_conflict_input(
    id: Option<&str>,
    direction: &str,
    listen: SocketAddr,
    peer: Option<&str>,
    spec: &RouteSpec,
) -> Result<RouteConflictInput> {
    Ok(RouteConflictInput {
        id: id.map(ToOwned::to_owned),
        direction: direction.to_string(),
        listen: listen.to_string(),
        peer: peer.map(ToOwned::to_owned),
        spec: route_spec_value(spec)?,
    })
}

fn route_spec_value(spec: &RouteSpec) -> Result<Value> {
    serde_json::to_value(spec).context("failed to serialize route spec for conflict policy")
}
