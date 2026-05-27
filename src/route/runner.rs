use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{cli, config, control_socket, node_daemon};

use super::{
    RemoteUsePlan, add_local_transport_probe_results, apply_local_forward_fallback,
    local_uses_remote_plan, node_forward_from_route, node_reverse_from_route,
    remote_direct_host_args, remote_use_decision, remote_uses_local_direct_plan,
    remote_uses_local_reverse_link_plan, route_id, route_intent_request,
};

pub async fn run(args: cli::RouteArgs, config: config::AppConfig) -> Result<()> {
    let endpoint = control_socket::ControlEndpoint::parse(&args.endpoint)?;
    if args.explain {
        let plan = explain_plan(&args, &config).await?;
        if args.json {
            println!("{}", serde_json::to_string(&plan)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(());
    }
    let mut request = route_intent_request(args.clone());
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&request)?);
        return Ok(());
    }
    if endpoint.is_tcp() {
        node_daemon::attach_auth_token(
            &mut request,
            args.token.as_deref().or(config.daemon.token.as_deref()),
        );
    }
    let response = control_socket::request(&endpoint, &format!("{request}\n"))
        .await
        .with_context(|| {
            format!(
                "failed to contact local daemon at {}; run `ssh_proxy daemon install --scope system --elevate` first",
                args.endpoint
            )
        })?;
    print!("{response}");
    Ok(())
}

pub(crate) async fn explain_plan(
    args: &cli::RouteArgs,
    config: &config::AppConfig,
) -> Result<Value> {
    match args.direction {
        cli::RouteDirection::LocalUsesRemote => {
            let mut forward = node_forward_from_route(args, config, args.target.clone(), false)?;
            let id = route_id(args, "local-via-remote");
            let mut plan = local_uses_remote_plan(args, &id, &forward);
            add_local_transport_probe_results(&mut plan, &mut forward).await;
            apply_local_forward_fallback(&mut forward, &mut plan);
            Ok(plan)
        }
        cli::RouteDirection::RemoteUsesLocal => {
            let decision = remote_use_decision(args, config)?;
            match decision.plan {
                RemoteUsePlan::ReverseLink => {
                    let reverse = node_reverse_from_route(args, config)?;
                    let id = route_id(args, "remote-via-local-reverse-link");
                    Ok(remote_uses_local_reverse_link_plan(
                        args,
                        &id,
                        &reverse,
                        decision.fallback_reason.as_deref(),
                    ))
                }
                RemoteUsePlan::Direct(local_peer) => {
                    let token = config
                        .daemon
                        .token
                        .clone()
                        .unwrap_or_else(|| "<daemon-token>".to_string());
                    let host_args = remote_direct_host_args(args, config, local_peer, token)?;
                    match &host_args.command {
                        cli::HostCommand::NodeForward(forward) => {
                            Ok(remote_uses_local_direct_plan(
                                args,
                                forward.id.as_deref().unwrap_or("remote-direct"),
                                forward,
                                local_peer,
                            ))
                        }
                        _ => bail!("unexpected remote direct route command"),
                    }
                }
            }
        }
    }
}
