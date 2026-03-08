use crate::discovery::{AppEntry, FileEntry};
use crate::frecency::FrequencyData;
use crate::{fuzzy_match, FuzzyMatch};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fuzziness {
    Strict,
    Balanced,
    Loose,
}

impl Fuzziness {
    pub fn label(self) -> &'static str {
        match self {
            Self::Strict => "Strict",
            Self::Balanced => "Balanced",
            Self::Loose => "Loose",
        }
    }

    pub fn more(self) -> Self {
        match self {
            Self::Strict => Self::Balanced,
            Self::Balanced => Self::Loose,
            Self::Loose => Self::Loose,
        }
    }

    pub fn less(self) -> Self {
        match self {
            Self::Strict => Self::Strict,
            Self::Balanced => Self::Strict,
            Self::Loose => Self::Balanced,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Apps,
    Files,
}

impl SearchMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Apps => "Apps",
            Self::Files => "Files",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Apps => Self::Files,
            Self::Files => Self::Apps,
        }
    }
}

pub enum ResultItem<'a> {
    App(&'a AppEntry),
    File(&'a FileEntry),
}

#[derive(Clone, Copy, Debug)]
pub enum ResultSource {
    App,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    Prefix,
    Contains,
    Fuzzy,
}

pub struct Scored {
    pub source: ResultSource,
    pub index: usize,
    pub m: FuzzyMatch,
    pub match_kind: MatchKind,
    pub frecency_bonus: i32,
}

pub fn filtered(
    app_entries: &[AppEntry],
    file_entries: &[FileEntry],
    query: &str,
    mode: SearchMode,
    fuzziness: Fuzziness,
    frecency: Option<&FrecencyConfig>,
) -> Vec<Scored> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let hint = extension_hint(query);

    let results: Vec<Scored> = match mode {
        SearchMode::Apps => app_entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let bonus = frecency_bonus_for(&entry.name, query, frecency);
                score_app(index, &entry.name, query, bonus)
            })
            .collect(),
        SearchMode::Files => file_entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| score_file(index, &entry.name, query, hint, fuzziness))
            .collect(),
    };
    sort_by_score(results)
}

pub fn filtered_from_candidates(
    app_entries: &[AppEntry],
    file_entries: &[FileEntry],
    candidates: &[Scored],
    query: &str,
    mode: SearchMode,
    fuzziness: Fuzziness,
    frecency: Option<&FrecencyConfig>,
) -> Vec<Scored> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let hint = extension_hint(query);

    let results: Vec<Scored> = match mode {
        SearchMode::Apps => candidates
            .iter()
            .filter(|candidate| matches!(candidate.source, ResultSource::App))
            .filter_map(|candidate| {
                let entry = app_entries.get(candidate.index)?;
                let bonus = frecency_bonus_for(&entry.name, query, frecency);
                score_app(candidate.index, &entry.name, query, bonus)
            })
            .collect(),
        SearchMode::Files => candidates
            .iter()
            .filter(|candidate| matches!(candidate.source, ResultSource::File))
            .filter_map(|candidate| {
                let entry = file_entries.get(candidate.index)?;
                score_file(candidate.index, &entry.name, query, hint, fuzziness)
            })
            .collect(),
    };
    sort_by_score(results)
}

pub struct FrecencyConfig<'a> {
    pub data: &'a FrequencyData,
    pub now: u64,
    pub half_life_days: f64,
    pub bonus_weight: i32,
}

fn frecency_bonus_for(name: &str, query: &str, config: Option<&FrecencyConfig>) -> i32 {
    let Some(cfg) = config else { return 0 };
    let key = name.to_lowercase();
    let raw = crate::frecency::frequency_bonus(
        &key,
        cfg.data,
        cfg.now,
        cfg.half_life_days,
        cfg.bonus_weight,
    );
    cap_frecency_bonus(raw, name, query)
}

fn score_app(index: usize, name: &str, query: &str, frecency_bonus: i32) -> Option<Scored> {
    let mut m = fuzzy_match(query, name)?;
    let match_kind = classify_match(name, query);
    m.score -= frecency_bonus;
    Some(Scored {
        source: ResultSource::App,
        index,
        m,
        match_kind,
        frecency_bonus,
    })
}

fn score_file(
    index: usize,
    name: &str,
    query: &str,
    hint: Option<&str>,
    fuzziness: Fuzziness,
) -> Option<Scored> {
    let mut m = fuzzy_match(query, name)?;
    let match_kind = classify_match(name, query);
    apply_extension_rule(name, hint, fuzziness, &mut m)?;
    Some(Scored {
        source: ResultSource::File,
        index,
        m,
        match_kind,
        frecency_bonus: 0,
    })
}

fn apply_extension_rule(
    name: &str,
    hint: Option<&str>,
    fuzziness: Fuzziness,
    m: &mut FuzzyMatch,
) -> Option<()> {
    let Some(hint) = hint else {
        return Some(());
    };
    let ext_match = matches_extension(name, hint);
    match fuzziness {
        Fuzziness::Strict if !ext_match => None,
        Fuzziness::Balanced if ext_match => {
            m.score -= 120;
            Some(())
        }
        Fuzziness::Balanced => {
            m.score += 120;
            Some(())
        }
        _ => Some(()),
    }
}

fn sort_by_score(mut results: Vec<Scored>) -> Vec<Scored> {
    results.sort_by_key(|s| s.m.score);
    results
}

fn cap_frecency_bonus(raw_bonus: i32, name: &str, query: &str) -> i32 {
    if raw_bonus <= 0 {
        return 0;
    }

    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 0;
    }

    let name = name.to_lowercase();
    let base_cap = (query.chars().count() as i32 * 20).clamp(40, 180);
    let cap = if name.starts_with(&query) {
        base_cap
    } else if contains_at_word_boundary(&name, &query) {
        base_cap
    } else if name.contains(&query) {
        base_cap / 2
    } else {
        base_cap / 3
    };

    raw_bonus.min(cap.max(0))
}

fn contains_at_word_boundary(name: &str, query: &str) -> bool {
    let name_chars: Vec<char> = name.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    if query_chars.len() > name_chars.len() {
        return false;
    }
    for start in 0..=name_chars.len() - query_chars.len() {
        if !query_chars
            .iter()
            .zip(name_chars[start..start + query_chars.len()].iter())
            .all(|(q, c)| q == c)
        {
            continue;
        }
        let end = start + query_chars.len();
        let at_word_start = start == 0 || matches!(name_chars[start - 1], ' ' | '-' | '_' | '/');
        let at_word_end =
            end == name_chars.len() || matches!(name_chars[end], ' ' | '-' | '_' | '/');
        if at_word_start && at_word_end {
            return true;
        }
    }
    false
}

fn classify_match(name: &str, query: &str) -> MatchKind {
    let q = query.trim().to_lowercase();
    let n = name.to_lowercase();

    if n.starts_with(&q) {
        return MatchKind::Prefix;
    }
    if n.contains(&q) {
        return MatchKind::Contains;
    }

    MatchKind::Fuzzy
}

fn extension_hint(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    let (_, suffix) = trimmed.rsplit_once('.')?;
    let hint = suffix.trim().trim_matches('"').trim_matches('\'');
    if hint.is_empty()
        || hint
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '\\')
    {
        return None;
    }
    Some(hint)
}

fn matches_extension(name: &str, hint: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(hint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::AppEntry as DesktopEntry;
    use proptest::prelude::*;
    use std::path::PathBuf;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]

        #[test]
        fn prop_files_mode_extension_hint_only_returns_matching_extensions(
            ext in "[a-z]{1,5}",
            other_ext in "[a-z]{1,5}",
            query_stem in "[a-z0-9]{0,8}"
        ) {
            prop_assume!(ext != other_ext);

            let apps: Vec<DesktopEntry> = Vec::new();
            let files = vec![
                FileEntry { name: format!("alpha.{ext}"), path: PathBuf::from("/tmp/alpha") },
                FileEntry { name: format!("beta.{other_ext}"), path: PathBuf::from("/tmp/beta") },
                FileEntry { name: "gamma".to_string(), path: PathBuf::from("/tmp/gamma") },
            ];
            let query = format!("{query_stem}.{ext}");
            let results = filtered(&apps, &files, &query, SearchMode::Files, Fuzziness::Strict, None);

            for result in results {
                prop_assert!(
                    matches!(result.source, ResultSource::File),
                    "non-file result returned in file mode"
                );
                let file = &files[result.index];
                prop_assert!(
                    matches_extension(&file.name, &ext),
                    "file '{}' should have extension '{}'",
                    file.name,
                    ext
                );
            }
        }
    }

    #[test]
    fn frecency_bonus_is_strongly_capped_for_non_contiguous_matches() {
        let capped = cap_frecency_bonus(2000, "Update Manager", "ter");
        assert_eq!(capped, 20);
    }

    #[test]
    fn frecency_bonus_allows_more_for_prefix_matches() {
        let capped = cap_frecency_bonus(2000, "Terminal", "ter");
        assert_eq!(capped, 60);
    }

    #[test]
    fn frecency_cap_word_boundary_gets_full_cap() {
        let capped = cap_frecency_bonus(2000, "Visual Studio Code", "code");
        let base_cap = (4 * 20_i32).clamp(40, 180);
        assert_eq!(capped, base_cap);
    }

    #[test]
    fn frecency_cap_mid_word_gets_halved() {
        let capped = cap_frecency_bonus(2000, "Barcode", "code");
        let base_cap = (4 * 20_i32).clamp(40, 180);
        assert_eq!(capped, base_cap / 2);
    }
}
