use crate::desktop_entry::DesktopEntry;
use crate::{fuzzy_match, FuzzyMatch};

use super::state::SearchMode;

pub struct Scored<'a> {
    pub entry: &'a DesktopEntry,
    pub m: FuzzyMatch,
}

pub fn filtered<'a>(entries: &'a [DesktopEntry], query: &str, mode: SearchMode) -> Vec<Scored<'a>> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let mut results: Vec<Scored<'_>> = match mode {
        SearchMode::Apps => entries
            .iter()
            .filter_map(|entry| fuzzy_match(query, &entry.name).map(|m| Scored { entry, m }))
            .collect(),
    };
    results.sort_by_key(|s| s.m.score);
    results
}
