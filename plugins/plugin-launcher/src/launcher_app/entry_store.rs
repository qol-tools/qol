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
            frecency,
            frecency_path,
        }
    }

    pub fn ensure_filtered(&mut self, state: &LauncherState) {
        let key = Self::filter_key(state);
        if self.cache_key.as_ref() == Some(&key) {
            return;
        }

        self.cache = match self.cache_key.as_ref() {
            Some(previous) if Self::can_incremental_filter(previous, &key) => {
                self.filtered_incremental(&state.query, state.mode, state.fuzziness)
            }
            _ => self.filtered_full(&state.query, state.mode, state.fuzziness),
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

    fn filtered_incremental(&self, query: &str, mode: SearchMode, fuzziness: Fuzziness) -> Vec<Scored> {
        let frecency = self.frecency_config();
        search::filtered_from_candidates(
            &self.app_entries,
            &self.file_entries,
            &self.cache,
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
