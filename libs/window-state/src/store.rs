use std::io;
use std::path::{Path, PathBuf};

use qol_runtime::probe;

use super::{is_valid_key, WindowState, SCHEMA_VERSION};

const STATE_DIR: &str = "window-state";
const EXTENSION: &str = "json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowStateStore {
    dir: PathBuf,
}

impl WindowStateStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn shared() -> Option<Self> {
        qol_config::data_subdir(STATE_DIR).map(Self::new)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn load(&self, key: &str) -> Option<WindowState> {
        let token = probe::token(key);
        let Some(path) = self.path(key) else {
            qol_runtime::probe!("WINDOW_STATE_LOAD", "key={token} outcome=invalid_key");
            log::warn!("window state load rejected invalid key {key:?}");
            return None;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                qol_runtime::probe!("WINDOW_STATE_LOAD", "key={token} outcome=missing");
                return None;
            }
            Err(error) => {
                qol_runtime::probe!("WINDOW_STATE_LOAD", "key={token} outcome=read_failed");
                log::warn!("failed to read {}: {error}", path.display());
                return None;
            }
        };
        let state: WindowState = match serde_json::from_slice(&bytes) {
            Ok(state) => state,
            Err(error) => {
                qol_runtime::probe!("WINDOW_STATE_LOAD", "key={token} outcome=corrupt");
                log::warn!("discarding corrupt {}: {error}", path.display());
                return None;
            }
        };
        if state.version != SCHEMA_VERSION || state.key != key {
            qol_runtime::probe!(
                "WINDOW_STATE_LOAD",
                "key={token} outcome=mismatch version={}",
                state.version
            );
            log::warn!("ignoring mismatched window state {}", path.display());
            return None;
        }
        qol_runtime::probe!("WINDOW_STATE_LOAD", "key={token} outcome=loaded");
        Some(state)
    }

    pub fn save(&self, state: &WindowState) -> io::Result<()> {
        let token = probe::token(&state.key);
        let result = self.write(state);
        match &result {
            Ok(()) => qol_runtime::probe!("WINDOW_STATE_SAVE", "key={token} outcome=saved"),
            Err(error) => {
                qol_runtime::probe!(
                    "WINDOW_STATE_SAVE",
                    "key={token} outcome=failed kind={:?}",
                    error.kind()
                );
                log::warn!("failed to save window state {:?}: {error}", state.key);
            }
        }
        result
    }

    fn write(&self, state: &WindowState) -> io::Result<()> {
        let path = self
            .path(&state.key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid window key"))?;
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
        qol_fs::atomic_write_durable(&path, &json)
    }

    pub fn list(&self) -> Vec<WindowState> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut states: Vec<WindowState> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != EXTENSION {
                    return None;
                }
                self.load(path.file_stem()?.to_str()?)
            })
            .collect();
        states.sort_by(|a, b| a.key.cmp(&b.key));
        states
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        is_valid_key(key).then(|| self.dir.join(format!("{key}.{EXTENSION}")))
    }
}
