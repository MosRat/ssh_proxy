use anyhow::{Context, Result};
use serde_json::Value;

use crate::{cli, config, control_socket};

use super::args::{proxy_args_from_node_forward, reverse_args_from_node_reverse};
use super::{NodeRequest, attach_auth_token};

pub(crate) async fn run(args: cli::NodeControlArgs, config: config::AppConfig) -> Result<()> {
    let endpoint = control_socket::ControlEndpoint::parse(&args.endpoint)?;
    let cli_token = args.token.as_deref();
    let auth_token = endpoint
        .is_tcp()
        .then_some(cli_token.or(config.daemon.token.as_deref()))
        .flatten();
    let request = match args.command {
        cli::NodeControlCommand::Status => NodeRequest::command("status")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Descriptor => NodeRequest::command("descriptor")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Shutdown => NodeRequest::command("shutdown")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Nodes => NodeRequest::command("nodes")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Jobs => NodeRequest::command("jobs")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::NodeEnsure { scope } => NodeRequest::node_ensure(scope)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::NodeStart { id } => NodeRequest::node_start(id)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::NodeStop { id } => NodeRequest::node_stop(id)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::NodeRestart { id } => NodeRequest::node_restart(id)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Links => NodeRequest::command("links")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Connect { profile } => NodeRequest::connect(profile)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Disconnect { profile } => NodeRequest::disconnect(profile)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Forward(args) => {
            let id = args
                .id
                .clone()
                .unwrap_or_else(|| format!("forward:{}->{}", args.listen, args.target));
            let persist = !args.volatile;
            let proxy = proxy_args_from_node_forward(args);
            NodeRequest::route_start_forward(id, persist, proxy)
                .with_auth_token(auth_token)
                .to_line()?
        }
        cli::NodeControlCommand::Reverse(args) => {
            let id = args
                .id
                .clone()
                .unwrap_or_else(|| format!("reverse:{}<-{}", args.remote_listen, args.target));
            let persist = !args.volatile;
            let reverse = reverse_args_from_node_reverse(args);
            NodeRequest::route_start_reverse(id, persist, reverse, None)
                .with_auth_token(auth_token)
                .to_line()?
        }
        cli::NodeControlCommand::RoutePlan(args) => NodeRequest::route_plan(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::StopRoute { id } => NodeRequest::route_stop(id)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::RestartRoute { id } => NodeRequest::route_restart(id)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Routes => NodeRequest::command("route_list")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Peers => NodeRequest::command("peer_list")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::TokenRotate => NodeRequest::command("token_rotate")
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerBootstrap(args) => NodeRequest::peer_bootstrap(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerEnsure(args) => NodeRequest::peer_ensure(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerUpdate(args) => NodeRequest::peer_update(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerRefresh(args) => NodeRequest::peer_refresh(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerDiff(args) => NodeRequest::peer_diff(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerReconcile(args) => NodeRequest::peer_reconcile(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerCheckVersion(args) => NodeRequest::peer_check_version(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerRotateToken(args) => NodeRequest::peer_rotate_token(args)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::PeerForget { alias } => NodeRequest::peer_forget(alias)
            .with_auth_token(auth_token)
            .to_line()?,
        cli::NodeControlCommand::Send { json } => {
            let mut value: Value =
                serde_json::from_str(&json).context("invalid node control JSON")?;
            attach_auth_token(&mut value, auth_token);
            format!("{}\n", serde_json::to_string(&value)?)
        }
    };
    let response = control_socket::request(&endpoint, &request).await?;
    print!("{response}");
    Ok(())
}
