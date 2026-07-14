use std::path::Path;

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use tracing::warn;

use crate::config;

use super::now_unix;

pub(super) fn load_store<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(store) => Ok(store),
            Err(err) => {
                warn!(path = %path.display(), error = %err, "quarantining corrupt daemon state store");
                quarantine_corrupt_store(path)?;
                Ok(T::default())
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub(super) fn save_store<T: Serialize>(path: &Path, store: &T) -> Result<()> {
    let text =
        serde_json::to_string_pretty(store).context("failed to encode daemon state store")?;
    config::save_text_file_private(path, &text)
}

fn quarantine_corrupt_store(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let stamp = now_unix();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    let quarantine = path.with_file_name(format!("{name}.corrupt-{stamp}"));
    std::fs::rename(path, quarantine)
        .with_context(|| format!("failed to quarantine corrupt store {}", path.display()))
}
