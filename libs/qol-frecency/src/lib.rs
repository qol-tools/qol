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
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, data: &FrequencyData) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(data) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                eprintln!("[frecency] failed to write {}: {}", path.display(), e);
            }
        }
        Err(e) => eprintln!("[frecency] failed to serialize: {}", e),
    }
}
