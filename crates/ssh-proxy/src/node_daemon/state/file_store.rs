use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, RwLock, RwLockReadGuard};

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

/// Serialize commits for one file. Readers see the last successful commit while I/O runs.
pub(in crate::node_daemon) struct PersistentStore<T> {
    path: PathBuf,
    value: Arc<RwLock<T>>,
    writer: Arc<Mutex<()>>,
}

impl<T: Serialize + DeserializeOwned + Default + Clone + Send + Sync + 'static> PersistentStore<T> {
    pub(in crate::node_daemon) fn load(path: PathBuf) -> Result<Self> {
        let value = load_store(&path)?;
        Ok(Self {
            path,
            value: Arc::new(RwLock::new(value)),
            writer: Arc::default(),
        })
    }

    pub(in crate::node_daemon) async fn read(&self) -> RwLockReadGuard<'_, T> {
        self.value.read().await
    }

    pub(in crate::node_daemon) async fn update<R>(
        &self,
        change: impl FnOnce(&mut T) -> R,
    ) -> Result<R> {
        let writer = self.writer.clone().lock_owned().await;
        let mut next = self.value.read().await.clone();
        let result = change(&mut next);
        let path = self.path.clone();
        let value = self.value.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            // Keep ownership of the commit even if the requesting task is cancelled.
            let _writer = writer;
            save_store(&path, &next)?;
            *value.blocking_write() = next;
            Ok(())
        })
        .await??;
        Ok(result)
    }
}
