use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::discovery;
use crate::frecency::{self, FrequencyData};

use super::search::{self, FrecencyConfig, Fuzziness, ResultItem, Scored, SearchMode};

const HALF_LIFE_DAYS: f64 = 7.0;
const FREQUENCY_BONUS: i32 = 500;
const MAX_FILTER_HISTORY: usize = 16;

#[derive(Clone, PartialEq, Eq)]
struct FilterKey {
    query: String,
    mode: SearchMode,
    fuzziness: Fuzziness,
}

pub struct EntryStore {
    app_entries: Arc<Vec<discovery::AppEntry>>,
    file_entries: Arc<Vec<discovery::FileEntry>>,
    cache: Vec<Scored>,
    cache_key: Option<FilterKey>,
    filter_history: Vec<(FilterKey, Vec<Scored>)>,
    frecency: FrequencyData,
    frecency_path: PathBuf,
    boosts: HashMap<String, i32>,
}

impl EntryStore {
    pub fn new(
        app_entries: Arc<Vec<discovery::AppEntry>>,
        file_entries: Arc<Vec<discovery::FileEntry>>,
    ) -> Self {
        let frecency_path = frecency::default_store_path("qol-launcher");
        let frecency = frecency::load(&frecency_path);
        let boosts = load_boosts(&frecency_path);
        Self {
            app_entries,
            file_entries,
            cache: Vec::new(),
            cache_key: None,
            filter_history: Vec::new(),
            frecency,
            frecency_path,
            boosts,
        }
    }

    pub fn ensure_filtered(&mut self, query: &str, mode: SearchMode, fuzziness: Fuzziness) {
        let key = FilterKey {
            query: query.to_owned(),
            mode,
            fuzziness,
        };
        if self.cache_key.as_ref() == Some(&key) {
            return;
        }

        if let Some(prev_key) = self.cache_key.take() {
            let prev_results = std::mem::take(&mut self.cache);
            if self.filter_history.len() >= MAX_FILTER_HISTORY {
                self.filter_history.remove(0);
            }
            self.filter_history.push((prev_key, prev_results));
        }

        if let Some(idx) = self.filter_history.iter().position(|(k, _)| k == &key) {
            let (_, results) = self.filter_history.remove(idx);
            self.cache = results;
            self.cache_key = Some(key);
            return;
        }

        let incremental = self
            .filter_history
            .last()
            .map(|(prev, _)| Self::can_incremental_filter(prev, &key))
            .unwrap_or(false);

        self.cache = if incremental {
            let frecency = self.frecency_config();
            search::filtered_from_candidates(
                &self.app_entries,
                &self.file_entries,
                &self.filter_history.last().unwrap().1,
                query,
                mode,
                fuzziness,
                Some(&frecency),
            )
        } else {
            self.filtered_full(query, mode, fuzziness)
        };
        self.cache_key = Some(key);
    }

    pub fn results(&self) -> &[Scored] {
        &self.cache
    }

    pub fn get(&self, index: usize) -> Option<&Scored> {
        self.cache.get(index)
    }

    pub fn result_count(&self) -> usize {
        self.cache.len()
    }

    pub fn replace_entries(
        &mut self,
        app_entries: Arc<Vec<discovery::AppEntry>>,
        file_entries: Arc<Vec<discovery::FileEntry>>,
    ) {
        if Arc::ptr_eq(&self.app_entries, &app_entries)
            && Arc::ptr_eq(&self.file_entries, &file_entries)
        {
            return;
        }
        self.app_entries = app_entries;
        self.file_entries = file_entries;
        self.cache.clear();
        self.cache_key = None;
        self.filter_history.clear();
    }

    pub fn name(&self, scored: &Scored) -> &str {
        match scored.source {
            search::ResultSource::App => &self.app_entries[scored.index].name,
            search::ResultSource::File => &self.file_entries[scored.index].name,
        }
    }

    pub fn item(&self, scored: &Scored) -> Option<ResultItem<'_>> {
        match scored.source {
            search::ResultSource::App => self.app_entries.get(scored.index).map(ResultItem::App),
            search::ResultSource::File => self.file_entries.get(scored.index).map(ResultItem::File),
        }
    }

    fn can_incremental_filter(previous: &FilterKey, next: &FilterKey) -> bool {
        previous.mode == next.mode
            && previous.fuzziness == next.fuzziness
            && !previous.query.is_empty()
            && next.query.starts_with(&previous.query)
    }

    pub fn adjust_boost(&mut self, name: &str, delta: i32) {
        let key = name.to_lowercase();
        let current = self.boosts.get(&key).copied().unwrap_or(0);
        let new_val = (current + delta).max(0);
        if new_val == 0 {
            self.boosts.remove(&key);
        } else {
            self.boosts.insert(key, new_val);
        }
        save_boosts(&self.frecency_path, &self.boosts);
    }

    pub fn invalidate_cache(&mut self) {
        self.cache.clear();
        self.cache_key = None;
        self.filter_history.clear();
    }

    pub fn record_launch(&mut self, name: &str) {
        let key = name.to_lowercase();
        let now = now_secs();
        frecency::record(&mut self.frecency, key, now);
        frecency::prune(&mut self.frecency, now, HALF_LIFE_DAYS);
        frecency::save(&self.frecency_path, &self.frecency);
    }

    fn frecency_config(&self) -> FrecencyConfig<'_> {
        FrecencyConfig {
            data: &self.frecency,
            now: now_secs(),
            half_life_days: HALF_LIFE_DAYS,
            bonus_weight: FREQUENCY_BONUS,
            boosts: &self.boosts,
        }
    }

    fn filtered_full(&self, query: &str, mode: SearchMode, fuzziness: Fuzziness) -> Vec<Scored> {
        let frecency = self.frecency_config();
        search::filtered(
            &self.app_entries,
            &self.file_entries,
            query,
            mode,
            fuzziness,
            Some(&frecency),
        )
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_boosts(frecency_path: &Path) -> HashMap<String, i32> {
    let boosts_path = frecency_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("qol-launcher-boosts.toml");
    let Ok(content) = std::fs::read_to_string(&boosts_path) else {
        return HashMap::new();
    };
    let Ok(table) = content.parse::<toml::Table>() else {
        return HashMap::new();
    };
    table
        .into_iter()
        .filter_map(|(k, v)| Some((k.to_lowercase(), v.as_integer()? as i32)))
        .collect()
}

fn save_boosts(frecency_path: &Path, boosts: &HashMap<String, i32>) {
    let boosts_path = frecency_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("qol-launcher-boosts.toml");
    let table: toml::Table = boosts
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::Integer(*v as i64)))
        .collect();
    let Ok(content) = toml::to_string(&table) else {
        return;
    };
    let _ = std::fs::write(&boosts_path, content);
}
