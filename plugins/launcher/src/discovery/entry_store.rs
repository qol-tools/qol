use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::discovery;
use crate::frecency::{self, FrequencyData};
use qol_plugin_api::launcher_flows::FlowEntry;

use super::search::{
    self, EntrySlices, FrecencyConfig, Fuzziness, MatchKind, ResultItem, ResultSource, Scored,
    SearchMode,
};
#[cfg(debug_assertions)]
use super::trace::{self, FilterSample};

const HALF_LIFE_DAYS: f64 = 7.0;
const FREQUENCY_BONUS: i32 = 500;
const MAX_FILTER_HISTORY: usize = 16;
const MOST_USED_LIMIT: usize = 8;
pub const RECENT_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BeforeTyping {
    pub apps: bool,
    pub files: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct FilterKey {
    query: String,
    mode: SearchMode,
    fuzziness: Fuzziness,
}

pub struct EntryStore {
    app_entries: Arc<Vec<discovery::AppEntry>>,
    file_entries: Arc<Vec<discovery::FileEntry>>,
    flow_entries: Arc<Vec<FlowEntry>>,
    cache: Vec<Scored>,
    cache_key: Option<FilterKey>,
    filter_history: Vec<(FilterKey, Vec<Scored>)>,
    frecency: FrequencyData,
    frecency_path: PathBuf,
    boosts: HashMap<String, i32>,
    recent_entries: Vec<discovery::FileEntry>,
    before_typing: BeforeTyping,
}

impl EntryStore {
    pub fn new(
        app_entries: Arc<Vec<discovery::AppEntry>>,
        file_entries: Arc<Vec<discovery::FileEntry>>,
        flow_entries: Arc<Vec<FlowEntry>>,
    ) -> Self {
        let frecency_path = frecency::default_store_path(qol_conventions::launcher::WINDOW_TITLE);
        let frecency = frecency::load(&frecency_path);
        let boosts = load_boosts(&frecency_path);
        Self {
            app_entries,
            file_entries,
            flow_entries,
            cache: Vec::new(),
            cache_key: None,
            filter_history: Vec::new(),
            frecency,
            frecency_path,
            boosts,
            recent_entries: Vec::new(),
            before_typing: BeforeTyping::default(),
        }
    }

    pub fn set_before_typing(&mut self, before_typing: BeforeTyping) {
        if self.before_typing != before_typing {
            self.before_typing = before_typing;
            self.invalidate_cache();
        }
    }

    pub fn set_recent_files(&mut self, paths: Vec<PathBuf>) {
        let recent: Vec<discovery::FileEntry> = paths
            .into_iter()
            .take(RECENT_LIMIT)
            .filter_map(|path| {
                let name = path.file_name()?.to_string_lossy().into_owned();
                Some(discovery::FileEntry { name, path })
            })
            .collect();
        if self.recent_entries != recent {
            self.recent_entries = recent;
            self.invalidate_cache();
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
        #[cfg(debug_assertions)]
        let started = std::time::Instant::now();
        #[cfg(debug_assertions)]
        trace::reset_counters();

        if let Some(prev_key) = self.cache_key.take() {
            let prev_results = std::mem::take(&mut self.cache);
            if self.filter_history.len() >= MAX_FILTER_HISTORY {
                self.filter_history.remove(0);
            }
            self.filter_history.push((prev_key, prev_results));
        }

        if let Some(idx) = self.filter_history.iter().position(|(k, _)| k == &key) {
            let (_, results) = self.filter_history.remove(idx);
            #[cfg(debug_assertions)]
            let result_count = results.len();
            self.cache = results;
            self.cache_key = Some(key);
            #[cfg(debug_assertions)]
            trace::filter(FilterSample {
                path: "history_cache",
                query,
                mode,
                fuzziness,
                app_count: self.app_entries.len(),
                file_count: self.file_entries.len(),
                candidate_count: result_count,
                result_count,
                elapsed_us: started.elapsed().as_micros(),
            });
            return;
        }

        let incremental = self
            .filter_history
            .last()
            .map(|(prev, _)| Self::can_incremental_filter(prev, &key))
            .unwrap_or(false);
        #[cfg(debug_assertions)]
        let (path, candidate_count) = if incremental {
            (
                "incremental",
                self.filter_history
                    .last()
                    .map(|(_, results)| results.len())
                    .unwrap_or(0),
            )
        } else {
            (
                "full",
                match mode {
                    SearchMode::Apps => self.app_entries.len(),
                    SearchMode::Files => self.file_entries.len(),
                },
            )
        };

        self.cache = if incremental {
            let frecency = self.frecency_config();
            search::filtered_from_candidates(
                EntrySlices {
                    apps: &self.app_entries,
                    files: &self.file_entries,
                    flows: &self.flow_entries,
                },
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
        #[cfg(debug_assertions)]
        trace::filter(FilterSample {
            path,
            query,
            mode,
            fuzziness,
            app_count: self.app_entries.len(),
            file_count: self.file_entries.len(),
            candidate_count,
            result_count: self.cache.len(),
            elapsed_us: started.elapsed().as_micros(),
        });
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
        flow_entries: Arc<Vec<FlowEntry>>,
    ) {
        if Arc::ptr_eq(&self.app_entries, &app_entries)
            && Arc::ptr_eq(&self.file_entries, &file_entries)
            && Arc::ptr_eq(&self.flow_entries, &flow_entries)
        {
            return;
        }
        self.app_entries = app_entries;
        self.file_entries = file_entries;
        self.flow_entries = flow_entries;
        self.cache.clear();
        self.cache_key = None;
        self.filter_history.clear();
    }

    pub fn name(&self, scored: &Scored) -> &str {
        match scored.source {
            ResultSource::App => &self.app_entries[scored.index].name,
            ResultSource::File => &self.file_entries[scored.index].name,
            ResultSource::Recent => &self.recent_entries[scored.index].name,
            ResultSource::Flow => &self.flow_entries[scored.index].title,
        }
    }

    pub fn item(&self, scored: &Scored) -> Option<ResultItem<'_>> {
        match scored.source {
            ResultSource::App => self.app_entries.get(scored.index).map(ResultItem::App),
            ResultSource::File => self.file_entries.get(scored.index).map(ResultItem::File),
            ResultSource::Recent => self.recent_entries.get(scored.index).map(ResultItem::File),
            ResultSource::Flow => self.flow_entries.get(scored.index).map(ResultItem::Flow),
        }
    }

    fn can_incremental_filter(previous: &FilterKey, next: &FilterKey) -> bool {
        previous.mode == next.mode
            && previous.fuzziness == next.fuzziness
            && !previous.query.is_empty()
            && next.query.starts_with(&previous.query)
    }

    pub fn adjust_selected_boost(
        &mut self,
        selected: usize,
        delta: i32,
        query: &str,
        mode: SearchMode,
        fuzziness: Fuzziness,
    ) -> Option<usize> {
        let scored = self.get(selected)?;
        if !matches!(scored.source, ResultSource::App) {
            return None;
        }
        let app_index = scored.index;
        let name = self.app_entries.get(app_index)?.name.clone();
        self.adjust_boost(&name, delta);
        self.invalidate_cache();
        self.ensure_filtered(query, mode, fuzziness);
        self.cache.iter().position(|result| {
            matches!(result.source, ResultSource::App) && result.index == app_index
        })
    }

    pub fn step_selected(
        &mut self,
        selected: usize,
        up: bool,
        query: &str,
        mode: SearchMode,
        fuzziness: Fuzziness,
    ) -> Option<usize> {
        let score = self.get(selected)?.m.score;
        let neighbour = if up {
            self.get(selected.checked_sub(1)?)
        } else {
            self.get(selected + 1)
        }?
        .m
        .score;
        let delta = if up {
            score - neighbour + 1
        } else {
            score - neighbour - 1
        };
        self.adjust_selected_boost(selected, delta, query, mode, fuzziness)
    }

    pub fn boost(&self, name: &str) -> i32 {
        self.boosts.get(&name.to_lowercase()).copied().unwrap_or(0)
    }

    fn adjust_boost(&mut self, name: &str, delta: i32) {
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
        if query.trim().is_empty() {
            return self.before_typing_results(mode);
        }
        let frecency = self.frecency_config();
        search::filtered(
            EntrySlices {
                apps: &self.app_entries,
                files: &self.file_entries,
                flows: &self.flow_entries,
            },
            query,
            mode,
            fuzziness,
            Some(&frecency),
        )
    }
}

impl EntryStore {
    fn before_typing_results(&self, mode: SearchMode) -> Vec<Scored> {
        match mode {
            SearchMode::Apps if self.before_typing.apps => self.most_used(),
            SearchMode::Files if self.before_typing.files => (0..self.recent_entries.len())
                .map(|index| unranked(ResultSource::Recent, index, index as i32, 0))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn most_used(&self) -> Vec<Scored> {
        let now = now_secs();
        let mut used: Vec<Scored> = self
            .app_entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let bonus = frecency::frequency_bonus(
                    &entry.name.to_lowercase(),
                    &self.frecency,
                    now,
                    HALF_LIFE_DAYS,
                    FREQUENCY_BONUS,
                );
                let boost = self
                    .boosts
                    .get(&entry.name.to_lowercase())
                    .copied()
                    .unwrap_or(0);
                (bonus > 0 || boost > 0).then(|| Scored {
                    manual_boost: boost,
                    ..unranked(ResultSource::App, index, -(bonus + boost), bonus)
                })
            })
            .collect();
        used.sort_by_key(|scored| scored.m.score);
        used.truncate(MOST_USED_LIMIT);
        used
    }
}

fn unranked(source: ResultSource, index: usize, score: i32, frecency_bonus: i32) -> Scored {
    Scored {
        source,
        index,
        m: crate::FuzzyMatch {
            score,
            positions: Vec::new(),
        },
        match_kind: MatchKind::Prefix,
        frecency_bonus,
        manual_boost: 0,
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
        .join(qol_conventions::launcher::BOOSTS_FILE_NAME);
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
        .join(qol_conventions::launcher::BOOSTS_FILE_NAME);
    let table: toml::Table = boosts
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::Integer(*v as i64)))
        .collect();
    let Ok(content) = toml::to_string(&table) else {
        return;
    };
    let _ = std::fs::write(&boosts_path, content);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn repeated_boosts_follow_the_same_app_through_reordering() {
        let temp = TempDir::new().unwrap();
        let mut store = EntryStore {
            app_entries: Arc::new(
                ["Qol Rank Alpha", "Qol Rank Alpine"]
                    .into_iter()
                    .map(|name| discovery::AppEntry {
                        name: name.to_string(),
                        exec: vec!["/usr/bin/true".to_string()],
                        path: PathBuf::from(format!("/{name}.desktop")),
                    })
                    .collect(),
            ),
            file_entries: Arc::new(Vec::new()),
            flow_entries: Arc::new(Vec::new()),
            cache: Vec::new(),
            cache_key: None,
            filter_history: Vec::new(),
            frecency: FrequencyData::default(),
            frecency_path: temp.path().join("frequency.json"),
            boosts: HashMap::new(),
            recent_entries: Vec::new(),
            before_typing: BeforeTyping::default(),
        };
        store.ensure_filtered("rank", SearchMode::Apps, Fuzziness::Balanced);
        assert_eq!(store.name(&store.results()[0]), "Qol Rank Alpha");
        let mut selected = 1;

        for expected_boost in [25, 50] {
            selected = store
                .adjust_selected_boost(selected, 25, "rank", SearchMode::Apps, Fuzziness::Balanced)
                .unwrap();
            assert_eq!(store.name(store.get(selected).unwrap()), "Qol Rank Alpine");
            assert_eq!(store.get(selected).unwrap().manual_boost, expected_boost);
            assert_eq!(
                load_boosts(&store.frecency_path)["qol rank alpine"],
                expected_boost
            );
            assert!(!store.boosts.contains_key("qol rank alpha"));
        }

        for expected_boost in [25, 0] {
            selected = store
                .adjust_selected_boost(selected, -25, "rank", SearchMode::Apps, Fuzziness::Balanced)
                .unwrap();
            assert_eq!(store.name(store.get(selected).unwrap()), "Qol Rank Alpine");
            assert_eq!(store.get(selected).unwrap().manual_boost, expected_boost);
        }
        assert!(load_boosts(&store.frecency_path).is_empty());

        let raised = store
            .step_selected(1, true, "rank", SearchMode::Apps, Fuzziness::Balanced)
            .unwrap();
        assert_eq!(raised, 0);
        assert_eq!(store.name(store.get(0).unwrap()), "Qol Rank Alpine");
        let lowered = store
            .step_selected(0, false, "rank", SearchMode::Apps, Fuzziness::Balanced)
            .unwrap();
        assert_eq!(lowered, 1);
        assert_eq!(store.name(store.get(1).unwrap()), "Qol Rank Alpine");
        assert!(store
            .step_selected(0, true, "rank", SearchMode::Apps, Fuzziness::Balanced)
            .is_none());
    }

    #[test]
    fn boosts_raise_apps_in_the_most_used_list() {
        let temp = TempDir::new().unwrap();
        let mut frecency = FrequencyData::default();
        let now = now_secs();
        for _ in 0..3 {
            frecency::record(&mut frecency, "qol rank alpha".to_string(), now);
        }
        frecency::record(&mut frecency, "qol rank alpine".to_string(), now);
        let mut store = EntryStore {
            app_entries: Arc::new(
                ["Qol Rank Alpha", "Qol Rank Alpine", "Qol Rank Unused"]
                    .into_iter()
                    .map(|name| discovery::AppEntry {
                        name: name.to_string(),
                        exec: vec!["/usr/bin/true".to_string()],
                        path: PathBuf::from(format!("/{name}.desktop")),
                    })
                    .collect(),
            ),
            file_entries: Arc::new(Vec::new()),
            flow_entries: Arc::new(Vec::new()),
            cache: Vec::new(),
            cache_key: None,
            filter_history: Vec::new(),
            frecency,
            frecency_path: temp.path().join("frequency.json"),
            boosts: HashMap::new(),
            recent_entries: Vec::new(),
            before_typing: BeforeTyping {
                apps: true,
                files: false,
            },
        };
        store.ensure_filtered("", SearchMode::Apps, Fuzziness::Balanced);
        assert_eq!(store.result_count(), 2);
        assert_eq!(store.name(&store.results()[0]), "Qol Rank Alpha");

        let mut selected = 1;
        while selected > 0 {
            selected = store
                .adjust_selected_boost(selected, 25, "", SearchMode::Apps, Fuzziness::Balanced)
                .unwrap();
        }
        assert_eq!(store.name(&store.results()[0]), "Qol Rank Alpine");
    }
}
