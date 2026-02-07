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

pub struct Scored<'a> {
    pub item: ResultItem<'a>,
    pub m: FuzzyMatch,
}

pub fn filtered<'a>(
    app_entries: &'a [AppEntry],
    file_entries: &'a [FileEntry],
    query: &str,
    mode: SearchMode,
    fuzziness: Fuzziness,
) -> Vec<Scored<'a>> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let extension_hint = extension_hint(query);

    let mut results: Vec<Scored<'_>> = match mode {
        SearchMode::Apps => app_entries
            .iter()
            .filter_map(|entry| {
                fuzzy_match(query, &entry.name).map(|m| Scored {
                    item: ResultItem::App(entry),
                    m,
                })
            })
            .collect(),
        SearchMode::Files => file_entries
            .iter()
            .filter_map(|entry| {
                let mut m = fuzzy_match(query, &entry.name)?;
                if let Some(hint) = extension_hint {
                    let ext_match = matches_extension(&entry.name, hint);
                    match fuzziness {
                        Fuzziness::Strict => {
                            if !ext_match {
                                return None;
                            }
                        }
                        Fuzziness::Balanced => {
                            if ext_match {
                                m.score -= 120;
                            } else {
                                m.score += 120;
                            }
                        }
                        Fuzziness::Loose => {}
                    }
                }
                Some(Scored {
                    item: ResultItem::File(entry),
                    m,
                })
            })
            .collect(),
    };
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
                let ResultItem::File(file) = result.item else {
                    prop_assert!(false, "non-file result returned in file mode");
                    continue;
                };
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
