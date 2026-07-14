use std::net::SocketAddr;

use anyhow::Result;

use crate::{cli, config, control_socket};

pub async fn control(mut args: cli::ControlArgs, config: config::AppConfig) -> Result<()> {
    let default_addr = SocketAddr::from(([127, 0, 0, 1], 1081));
    if args.endpoint.is_none()
        && args.addr == default_addr
        && let Some(addr) = config.daemon.control_listen
    {
        args.addr = addr;
    }
    let endpoint = match args
        .endpoint
        .as_deref()
        .or(config.daemon.control_endpoint.as_deref())
    {
        Some(value) => control_socket::ControlEndpoint::parse(value)?,
        None => control_socket::ControlEndpoint::from_addr(args.addr),
    };
    let command = match args.command {
        cli::ControlCommand::Status => "status\n".to_string(),
        cli::ControlCommand::Shutdown => "shutdown\n".to_string(),
        cli::ControlCommand::Connect { profile } => {
            format!(
                "{}\n",
                serde_json::json!({"cmd": "connect", "profile": profile})
            )
        }
        cli::ControlCommand::Disconnect { profile } => {
            format!(
                "{}\n",
                serde_json::json!({"cmd": "disconnect", "profile": profile})
            )
        }
    };
    let response = control_socket::request(&endpoint, &command).await?;
    print!("{response}");
    Ok(())
}
