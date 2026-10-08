use std::io;
use std::path::{Path, PathBuf};

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
        let bytes = std::fs::read(self.path(key)?).ok()?;
        let state: WindowState = serde_json::from_slice(&bytes).ok()?;
        (state.version == SCHEMA_VERSION && state.key == key).then_some(state)
    }

    pub fn save(&self, state: &WindowState) -> io::Result<()> {
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
