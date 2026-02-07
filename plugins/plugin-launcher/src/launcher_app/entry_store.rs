use crate::providers::{apps, files};

use super::search::{self, ResultItem, Scored};
use super::state::{Fuzziness, LauncherState, SearchMode};

#[derive(Clone, PartialEq, Eq)]
struct FilterKey {
    query: String,
    mode: SearchMode,
    fuzziness: Fuzziness,
}

pub(super) struct EntryStore {
    app_entries: Vec<apps::AppEntry>,
    file_entries: Vec<files::FileEntry>,
    cache: Vec<Scored>,
    cache_key: Option<FilterKey>,
}

impl EntryStore {
    pub fn new(app_entries: Vec<apps::AppEntry>, file_entries: Vec<files::FileEntry>) -> Self {
        Self {
            app_entries,
            file_entries,
            cache: Vec::new(),
            cache_key: None,
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

    fn filtered_full(&self, query: &str, mode: SearchMode, fuzziness: Fuzziness) -> Vec<Scored> {
        search::filtered(&self.app_entries, &self.file_entries, query, mode, fuzziness)
    }

    fn filtered_incremental(&self, query: &str, mode: SearchMode, fuzziness: Fuzziness) -> Vec<Scored> {
        search::filtered_from_candidates(
            &self.app_entries,
            &self.file_entries,
            &self.cache,
            query,
            mode,
            fuzziness,
        )
    }
}
