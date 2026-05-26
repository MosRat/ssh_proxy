use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use russh::{
    client::{self, AuthResult},
    keys::{self, PrivateKeyWithHashAlg, agent::client::AgentClient},
};
use tracing::{debug, info, warn};

use crate::ssh_client::{ClientHandler, Target};

pub(crate) async fn authenticate(
    session: &mut client::Handle<ClientHandler>,
    target: &Target,
) -> Result<()> {
    if try_agent(session, &target.user).await? {
        return Ok(());
    }

    let identities = if target.identities.is_empty() {
        default_identities()
    } else {
        target.identities.clone()
    };

    for path in identities {
        match load_private_key(&path).await {
            Ok(key) => {
                let hash_alg = session
                    .best_supported_rsa_hash()
                    .await
                    .ok()
                    .flatten()
                    .flatten();
                let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
                match session
                    .authenticate_publickey(target.user.clone(), key)
                    .await
                {
                    Ok(AuthResult::Success) => {
                        info!(path = %path.display(), "authenticated with private key");
                        return Ok(());
                    }
                    Ok(AuthResult::Failure {
                        remaining_methods, ..
                    }) => {
                        debug!(path = %path.display(), ?remaining_methods, "private key rejected");
                    }
                    Err(err) => {
                        warn!(path = %path.display(), error = %err, "private key auth failed")
                    }
                }
            }
            Err(err) => debug!(path = %path.display(), error = %err, "private key not usable"),
        }
    }

    match session.authenticate_none(target.user.clone()).await {
        Ok(AuthResult::Success) => return Ok(()),
        Ok(_) | Err(_) => {}
    }

    bail!("SSH authentication failed: no accepted agent identity or unencrypted identity file");
}

async fn try_agent(session: &mut client::Handle<ClientHandler>, user: &str) -> Result<bool> {
    #[cfg(unix)]
    {
        let mut agent = match AgentClient::connect_env().await {
            Ok(agent) => agent,
            Err(err) => {
                debug!(error = %err, "ssh-agent unavailable");
                return Ok(false);
            }
        };
        return try_agent_client(session, user, &mut agent).await;
    }

    #[cfg(windows)]
    {
        if let Ok(mut agent) = AgentClient::connect_pageant().await {
            match try_agent_client(session, user, &mut agent).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(err) => {
                    warn!(error = %err, "Pageant agent auth failed; trying other identities")
                }
            }
        }
        let pipe = r"\\.\pipe\openssh-ssh-agent";
        if let Ok(mut agent) = AgentClient::connect_named_pipe(pipe).await {
            match try_agent_client(session, user, &mut agent).await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    warn!(error = %err, "OpenSSH agent auth failed; trying identity files")
                }
            }
        }
        Ok(false)
    }
}

async fn try_agent_client<S>(
    session: &mut client::Handle<ClientHandler>,
    user: &str,
    agent: &mut AgentClient<S>,
) -> Result<bool>
where
    S: keys::agent::client::AgentStream + Unpin + Send + 'static,
{
    let identities = agent.request_identities().await?;
    for identity in identities {
        let public_key = identity.public_key().into_owned();
        let hash_alg = session
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten();
        match session
            .authenticate_publickey_with(user.to_string(), public_key, hash_alg, agent)
            .await
        {
            Ok(AuthResult::Success) => {
                info!(comment = %identity.comment(), "authenticated with ssh-agent");
                return Ok(true);
            }
            Ok(AuthResult::Failure {
                remaining_methods, ..
            }) => {
                debug!(comment = %identity.comment(), ?remaining_methods, "agent identity rejected");
            }
            Err(err) => warn!(comment = %identity.comment(), error = %err, "agent auth failed"),
        }
    }
    Ok(false)
}

async fn load_private_key(path: &Path) -> Result<russh::keys::ssh_key::PrivateKey> {
    let secret = tokio::fs::read_to_string(path).await?;
    let key = keys::decode_secret_key(&secret, None).context("failed to decode private key")?;
    if key.is_encrypted() {
        bail!("encrypted private key requires ssh-agent support");
    }
    Ok(key)
}

fn default_identities() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .into_iter()
        .map(|name| home.join(".ssh").join(name))
        .collect()
}
