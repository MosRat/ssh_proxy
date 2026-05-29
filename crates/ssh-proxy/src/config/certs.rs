use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::cli;

use super::{AppConfig, certs_dir, expand_path, io, profile};

pub(super) fn import(
    config: &mut AppConfig,
    args: cli::ConfigCertImportArgs,
) -> Result<serde_json::Value> {
    let store = certs_dir()?.join(sanitize_store_name(&args.name)?);
    std::fs::create_dir_all(&store)
        .with_context(|| format!("failed to create {}", store.display()))?;
    let remote_ca = copy_cert_arg(&store, "remote-ca.pem", args.remote_ca, args.overwrite)?;
    let client_cert = copy_cert_arg(&store, "client.pem", args.client_cert, args.overwrite)?;
    let client_key = copy_key_arg(&store, "client-key.pem", args.client_key, args.overwrite)?;
    let tls_cert = copy_cert_arg(&store, "tls.pem", args.tls_cert, args.overwrite)?;
    let tls_key = copy_key_arg(&store, "tls-key.pem", args.tls_key, args.overwrite)?;
    let tls_client_ca = copy_cert_arg(
        &store,
        "tls-client-ca.pem",
        args.tls_client_ca,
        args.overwrite,
    )?;

    if let Some(profile_name) = &args.profile {
        let profile = config.profiles.entry(profile_name.clone()).or_default();
        profile::set_opt(&mut profile.remote_ca, remote_ca.clone());
        profile::set_opt(&mut profile.remote_client_cert, client_cert.clone());
        profile::set_opt(&mut profile.remote_client_key, client_key.clone());
    }
    if args.daemon {
        profile::set_opt(&mut config.daemon.tls_cert, tls_cert.clone());
        profile::set_opt(&mut config.daemon.tls_key, tls_key.clone());
        profile::set_opt(&mut config.daemon.tls_client_ca, tls_client_ca.clone());
    }

    Ok(serde_json::json!({
        "ok": true,
        "store": store,
        "profile": args.profile,
        "daemon": args.daemon,
        "remote_ca": remote_ca,
        "client_cert": client_cert,
        "client_key": client_key,
        "tls_cert": tls_cert,
        "tls_key": tls_key,
        "tls_client_ca": tls_client_ca,
    }))
}

pub(super) fn copy_cert_arg(
    store: &Path,
    name: &str,
    src: Option<PathBuf>,
    overwrite: bool,
) -> Result<Option<PathBuf>> {
    copy_store_file(store, name, src, overwrite, false)
}

fn copy_key_arg(
    store: &Path,
    name: &str,
    src: Option<PathBuf>,
    overwrite: bool,
) -> Result<Option<PathBuf>> {
    copy_store_file(store, name, src, overwrite, true)
}

fn copy_store_file(
    store: &Path,
    name: &str,
    src: Option<PathBuf>,
    overwrite: bool,
    secret: bool,
) -> Result<Option<PathBuf>> {
    let Some(src) = src else {
        return Ok(None);
    };
    let src = expand_path(&src);
    let dst = store.join(name);
    if dst.exists() && !overwrite {
        bail!(
            "{} already exists; pass --overwrite to replace it",
            dst.display()
        );
    }
    std::fs::copy(&src, &dst)
        .with_context(|| format!("failed to copy {} to {}", src.display(), dst.display()))?;
    io::set_file_private(&dst, secret)?;
    Ok(Some(dst))
}

pub(super) fn sanitize_store_name(name: &str) -> Result<String> {
    let value = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('.').to_string();
    if value.is_empty() {
        bail!("certificate store name cannot be empty");
    }
    Ok(value)
}
