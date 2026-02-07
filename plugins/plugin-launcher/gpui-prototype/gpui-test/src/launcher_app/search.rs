use crate::desktop_entry::DesktopEntry;
use crate::providers::files::FileEntry;
use crate::{fuzzy_match, FuzzyMatch};

use super::state::SearchMode;

pub enum ResultItem<'a> {
    App(&'a DesktopEntry),
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
    app_entries: &'a [DesktopEntry],
    file_entries: &'a [FileEntry],
    query: &str,
    mode: SearchMode,
) -> Vec<Scored<'a>> {
    if query.trim().is_empty() {
        return Vec::new();
    }

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
                fuzzy_match(query, &entry.name).map(|m| Scored {
                    item: ResultItem::File(entry),
                    m,
                })
            })
            .collect(),
    };
    results.sort_by_key(|s| s.m.score);
    results
}
