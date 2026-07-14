use anyhow::Result;

use crate::{cli, config, peer_lifecycle};

use super::proxy_session::ProxySessionSpec;

pub(super) fn install_args_from_spec(
    config: &config::AppConfig,
    spec: &ProxySessionSpec,
) -> Result<cli::InstallRemoteArgs> {
    let mut args = peer_lifecycle::spec::install_args_from_proxy_session(Some(config), spec)?;
    crate::config::apply_install_defaults(config, &mut args, Some(&spec.target))?;
    if let Some(profile) = config.profiles.get(&spec.target) {
        args.target = profile
            .target
            .clone()
            .unwrap_or_else(|| spec.target.clone());
    }
    Ok(args)
}
