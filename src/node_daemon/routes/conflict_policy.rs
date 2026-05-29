use std::{collections::HashMap, net::SocketAddr};

use anyhow::{Context, Result, bail};

use crate::cli;

use super::{RouteSpec, RouteTask};

pub(super) fn ensure_new_route_can_start(
    routes: &HashMap<String, RouteTask>,
    direction: &str,
    listen: SocketAddr,
    peer: Option<&str>,
    spec: &RouteSpec,
) -> Result<()> {
    ensure_listener_not_reserved(routes, direction, listen, peer)?;
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
    task.direction == direction
        && task.listen == Some(listen)
        && match peer {
            Some(peer) => task.peer.as_deref() == Some(peer),
            None => true,
        }
        && route_specs_match(&task.spec, spec)
}

pub(super) fn route_specs_match(left: &RouteSpec, right: &RouteSpec) -> bool {
    match (left, right) {
        (RouteSpec::Reverse { reverse: left }, RouteSpec::Reverse { reverse: right }) => {
            reverse_route_specs_match(left, right)
        }
        _ => serde_json::to_value(left).ok() == serde_json::to_value(right).ok(),
    }
}

fn ensure_port_available(addr: SocketAddr) -> Result<()> {
    let listener = std::net::TcpListener::bind(addr)
        .with_context(|| format!("listen address {addr} is already in use or unavailable"))?;
    drop(listener);
    Ok(())
}

fn ensure_listener_not_reserved(
    routes: &HashMap<String, RouteTask>,
    direction: &str,
    listen: SocketAddr,
    peer: Option<&str>,
) -> Result<()> {
    for (id, task) in routes {
        if task.direction != direction || task.listen != Some(listen) {
            continue;
        }
        if direction == "forward" || peer.is_some_and(|peer| task.peer.as_deref() == Some(peer)) {
            bail!("route {id:?} already owns {direction} listener {listen}");
        }
    }
    Ok(())
}

fn reverse_route_specs_match(left: &cli::ReverseTaskArgs, right: &cli::ReverseTaskArgs) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.identity.clear();
    right.identity.clear();
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}
