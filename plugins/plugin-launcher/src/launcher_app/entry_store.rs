use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::frecency::{self, FrequencyData};
use crate::frecency_store;
use crate::providers::{apps, files};

use super::search::{self, FrecencyConfig, ResultItem, Scored};
use super::state::{Fuzziness, LauncherState, SearchMode};

const HALF_LIFE_DAYS: f64 = 7.0;
const FREQUENCY_BONUS: i32 = 500;
const MAX_FILTER_HISTORY: usize = 16;

#[derive(Clone, PartialEq, Eq)]
struct FilterKey {
    query: String,
    mode: SearchMode,
    fuzziness: Fuzziness,
}

pub(super) struct EntryStore {
    app_entries: Arc<Vec<apps::AppEntry>>,
    file_entries: Arc<Vec<files::FileEntry>>,
    cache: Vec<Scored>,
    cache_key: Option<FilterKey>,
    filter_history: Vec<(FilterKey, Vec<Scored>)>,
    frecency: FrequencyData,
    frecency_path: PathBuf,
}

impl EntryStore {
    pub fn new(app_entries: Arc<Vec<apps::AppEntry>>, file_entries: Arc<Vec<files::FileEntry>>) -> Self {
        let frecency_path = frecency_store::default_path();
        let frecency = frecency_store::load(&frecency_path);
        Self {
            app_entries,
            file_entries,
            cache: Vec::new(),
            cache_key: None,
            filter_history: Vec::new(),
            frecency,
            frecency_path,
        }
    }

    pub fn ensure_filtered(&mut self, state: &LauncherState) {
        let key = Self::filter_key(state);
        if self.cache_key.as_ref() == Some(&key) {
            return;
        }

        // Save current result to history before replacing
        if let Some(prev_key) = self.cache_key.take() {
            let prev_results = std::mem::take(&mut self.cache);
            if self.filter_history.len() >= MAX_FILTER_HISTORY {
                self.filter_history.remove(0);
            }
            self.filter_history.push((prev_key, prev_results));
        }

        // Check history for exact match
        if let Some(idx) = self.filter_history.iter().position(|(k, _)| k == &key) {
            let (_, results) = self.filter_history.remove(idx);
            self.cache = results;
            self.cache_key = Some(key);
            return;
        }

        // Check if incremental from the previous (last history entry)
        let incremental = self.filter_history.last()
            .map(|(prev, _)| Self::can_incremental_filter(prev, &key))
            .unwrap_or(false);

        self.cache = if incremental {
            let frecency = self.frecency_config();
            search::filtered_from_candidates(
                &self.app_entries,
                &self.file_entries,
                &self.filter_history.last().unwrap().1,
                &state.query,
                state.mode,
                state.fuzziness,
                Some(&frecency),
            )
        } else {
            self.filtered_full(&state.query, state.mode, state.fuzziness)
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
        app_entries: Arc<Vec<apps::AppEntry>>,
        file_entries: Arc<Vec<files::FileEntry>>,
    ) {
        if Arc::ptr_eq(&self.app_entries, &app_entries) && Arc::ptr_eq(&self.file_entries, &file_entries) {
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

    fn filter_key(state: &LauncherState) -> FilterKey {
        FilterKey {
            query: state.query.clone(),
            mode: state.mode,
            fuzziness: state.fuzziness,
        }
    }

    fn can_incremental_filter(previous: &FilterKey, next: &FilterKey) -> bool {
        previous.mode == next.mode
            && previous.fuzziness == next.fuzziness
            && !previous.query.is_empty()
            && next.query.starts_with(&previous.query)
    }

    pub fn record_launch(&mut self, name: &str) {
        let key = name.to_lowercase();
        let now = now_secs();
        frecency::record(&mut self.frecency, key, now);
        frecency::prune(&mut self.frecency, now, HALF_LIFE_DAYS);
        frecency_store::save(&self.frecency_path, &self.frecency);
    }

    fn frecency_config(&self) -> FrecencyConfig<'_> {
        FrecencyConfig {
            data: &self.frecency,
            now: now_secs(),
            half_life_days: HALF_LIFE_DAYS,
            bonus_weight: FREQUENCY_BONUS,
        }
    }

    fn filtered_full(&self, query: &str, mode: SearchMode, fuzziness: Fuzziness) -> Vec<Scored> {
        let frecency = self.frecency_config();
        search::filtered(&self.app_entries, &self.file_entries, query, mode, fuzziness, Some(&frecency))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
