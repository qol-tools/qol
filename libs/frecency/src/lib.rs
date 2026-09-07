use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SECS_PER_DAY: f64 = 86400.0;
const LN2: f64 = 0.693;
const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrequencyEntry {
    pub count: u32,
    pub last_accessed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrequencyData {
    pub entries: HashMap<String, FrequencyEntry>,
}

pub fn effective_count(entry: &FrequencyEntry, now: u64, half_life_days: f64) -> f64 {
    let days_elapsed = now.saturating_sub(entry.last_accessed) as f64 / SECS_PER_DAY;
    let decay = (-days_elapsed * LN2 / half_life_days).exp();
    entry.count as f64 * decay
}

pub fn frequency_bonus(
    key: &str,
    data: &FrequencyData,
    now: u64,
    half_life_days: f64,
    bonus_weight: i32,
) -> i32 {
    data.entries
        .get(key)
        .map(|e| (effective_count(e, now, half_life_days) * bonus_weight as f64) as i32)
        .unwrap_or(0)
}

pub fn record(data: &mut FrequencyData, key: String, now: u64) {
    let entry = data.entries.entry(key).or_insert(FrequencyEntry {
        count: 0,
        last_accessed: now,
    });
    entry.count += 1;
    entry.last_accessed = now;
}

pub fn prune(data: &mut FrequencyData, now: u64, half_life_days: f64) {
    let threshold = 0.01;
    data.entries
        .retain(|_, entry| effective_count(entry, now, half_life_days) >= threshold);
    if data.entries.len() > MAX_ENTRIES {
        let mut entries: Vec<_> = data.entries.drain().collect();
        entries.sort_by(|a, b| {
            let score_a = effective_count(&a.1, now, half_life_days);
            let score_b = effective_count(&b.1, now, half_life_days);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(MAX_ENTRIES);
        data.entries = entries.into_iter().collect();
    }
}

/// Returns the default frecency store path for a plugin.
///
/// Resolves to `$XDG_CACHE_DIR/{plugin_name}-frequency.json`.
pub fn default_store_path(plugin_name: &str) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(format!("{}-frequency.json", plugin_name))
}

pub fn load(path: &Path) -> FrequencyData {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FrequencyData::default(),
        Err(e) => {
            eprintln!("[frecency] failed to read {}: {}", path.display(), e);
            return FrequencyData::default();
        }
    };
    match serde_json::from_str(&contents) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("[frecency] discarding corrupt {}: {}", path.display(), e);
            FrequencyData::default()
        }
    }
}

pub fn save(path: &Path, data: &FrequencyData) {
    let json = match serde_json::to_string_pretty(data) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("[frecency] failed to serialize: {}", e);
            return;
        }
    };
    if let Err(e) = qol_fs::atomic_write(path, json.as_bytes()) {
        eprintln!("[frecency] failed to save {}: {}", path.display(), e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("foo-frequency.json");
        let mut data = FrequencyData::default();
        record(&mut data, "alpha".to_string(), 100);
        record(&mut data, "alpha".to_string(), 200);
        record(&mut data, "beta".to_string(), 300);

        save(&path, &data);
        let loaded = load(&path);

        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries["alpha"].count, 2);
        assert_eq!(loaded.entries["alpha"].last_accessed, 200);
        assert_eq!(loaded.entries["beta"].count, 1);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let loaded = load(&tmp.path().join("absent.json"));
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn load_corrupt_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("corrupt-frequency.json");
        std::fs::write(&path, "{ this is not valid json").unwrap();
        let loaded = load(&path);
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("foo-frequency.json");
        let mut data = FrequencyData::default();
        record(&mut data, "alpha".to_string(), 100);
        save(&path, &data);

        let leftovers: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".foo-frequency.json.") && n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {:?}", leftovers);
        assert!(path.exists());
    }
}
