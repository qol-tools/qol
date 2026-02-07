use crate::providers::apps::AppEntry;
use crate::providers::files::FileEntry;
use crate::{fuzzy_match, FuzzyMatch};
use std::path::Path;

use super::state::{Fuzziness, SearchMode};

pub enum ResultItem<'a> {
    App(&'a AppEntry),
    File(&'a FileEntry),
}

impl<'a> ResultItem<'a> {
    pub fn name(&self) -> &str {
        match self {
            Self::App(entry) => &entry.name,
            Self::File(entry) => &entry.name,
        }
    }
}

#[derive(Clone, Copy)]
pub enum ResultSource {
    App,
    File,
}

pub struct Scored {
    pub source: ResultSource,
    pub index: usize,
    pub m: FuzzyMatch,
}


pub fn filtered(
    app_entries: &[AppEntry],
    file_entries: &[FileEntry],
    query: &str,
    mode: SearchMode,
    fuzziness: Fuzziness,
) -> Vec<Scored> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let hint = extension_hint(query);

    let results: Vec<Scored> = match mode {
        SearchMode::Apps => app_entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| score_app(index, &entry.name, query))
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
                score_app(candidate.index, &entry.name, query)
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

fn score_app(index: usize, name: &str, query: &str) -> Option<Scored> {
    Some(Scored {
        source: ResultSource::App,
        index,
        m: fuzzy_match(query, name)?,
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
    apply_extension_rule(name, hint, fuzziness, &mut m)?;
    Some(Scored {
        source: ResultSource::File,
        index,
        m,
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

fn extension_hint(query: &str) -> Option<&str> {
    let trimmed = query.trim();
    let (_, suffix) = trimmed.rsplit_once('.')?;
    let hint = suffix.trim().trim_matches('"').trim_matches('\'');
    if hint.is_empty() || hint.chars().any(|c| c.is_whitespace() || c == '/' || c == '\\') {
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
    use crate::desktop_entry::DesktopEntry;
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
            let results = filtered(&apps, &files, &query, SearchMode::Files, Fuzziness::Strict);

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
}
