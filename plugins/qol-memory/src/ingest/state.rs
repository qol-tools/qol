use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::store::Store;

pub const SCHEMA: &str = "qol-memory-ingest-state-v1";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileState {
    pub offset: u64,
    pub size: u64,
    pub mtime_ms: u64,
    pub inode: u64,
    #[serde(default)]
    pub head: String,
    pub session: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Default)]
pub struct IngestState {
    files: BTreeMap<PathBuf, FileState>,
}

#[derive(serde::Deserialize)]
struct StateFile {
    schema: String,
    #[serde(default)]
    files: BTreeMap<PathBuf, FileState>,
}

impl IngestState {
    pub fn load(store: &Store) -> IngestState {
        let Ok(text) = std::fs::read_to_string(store.ingest_state_path()) else {
            return IngestState::default();
        };
        let Ok(parsed) = serde_json::from_str::<StateFile>(&text) else {
            return IngestState::default();
        };
        if parsed.schema != SCHEMA {
            return IngestState::default();
        }
        IngestState {
            files: parsed.files,
        }
    }

    pub fn get(&self, path: &Path) -> Option<&FileState> {
        self.files.get(path)
    }

    pub fn set(&mut self, path: &Path, state: FileState) {
        self.files.insert(path.to_path_buf(), state);
    }

    pub fn save(&self, store: &Store) -> anyhow::Result<()> {
        let mut document = serde_json::Map::new();
        document.insert("schema".to_string(), json!(SCHEMA));
        document.insert("files".to_string(), serde_json::to_value(&self.files)?);
        let mut text = serde_json::to_string_pretty(&Value::Object(document))?;
        text.push('\n');
        qol_fs::atomic_write(&store.ingest_state_path(), text.as_bytes())?;
        Ok(())
    }
}

pub fn inode_of(metadata: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        std::os::unix::fs::MetadataExt::ino(metadata)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

pub const HEAD_BYTES: u64 = 256;

pub fn head_of(path: &Path) -> String {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut head = Vec::new();
    if file.take(HEAD_BYTES).read_to_end(&mut head).is_err() {
        return String::new();
    }
    head.iter().map(|byte| format!("{byte:02x}")).collect()
}
