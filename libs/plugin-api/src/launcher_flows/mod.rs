use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

pub const FLOWS_FILE_NAME: &str = "launcher-flows.json";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct FlowEntry {
    pub plugin_id: String,
    pub title: String,
    pub prompt: String,
    pub query: String,
    #[serde(default)]
    pub row_actions: Vec<qol_config::contract::RowActionSpec>,
}

pub fn flows_path() -> Option<PathBuf> {
    qol_config::data_dir().map(|dir| dir.join(FLOWS_FILE_NAME))
}

pub fn write_flows(path: &Path, entries: &[FlowEntry]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let json = serde_json::to_vec_pretty(entries)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

pub fn read_flows(path: &Path) -> Vec<FlowEntry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qol-plugin-api-launcher-flows-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample_entries() -> Vec<FlowEntry> {
        vec![FlowEntry {
            plugin_id: "qol-memory".to_string(),
            title: "qol memory".to_string(),
            prompt: "Ask memory".to_string(),
            query: "rows".to_string(),
            row_actions: Vec::new(),
        }]
    }

    #[test]
    fn flows_round_trip_through_a_temp_file() {
        let dir = temp_path("round-trip");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FLOWS_FILE_NAME);
        let entries = sample_entries();

        write_flows(&path, &entries).unwrap();

        assert_eq!(read_flows(&path), entries);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_flows_tolerates_missing_and_garbage() {
        let dir = temp_path("missing-and-garbage");
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.json");
        let garbage = dir.join("garbage.json");
        std::fs::write(&garbage, b"{ not json").unwrap();

        assert!(read_flows(&missing).is_empty());
        assert!(read_flows(&garbage).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
