use crate::desktop_entry::DesktopEntry;
use crate::{fuzzy_match, FuzzyMatch};

pub struct Scored<'a> {
    pub entry: &'a DesktopEntry,
    pub m: FuzzyMatch,
}

pub fn filtered<'a>(entries: &'a [DesktopEntry], query: &str) -> Vec<Scored<'a>> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let mut results: Vec<Scored<'_>> = entries
        .iter()
        .filter_map(|entry| fuzzy_match(query, &entry.name).map(|m| Scored { entry, m }))
        .collect();
    results.sort_by_key(|s| s.m.score);
    results
}
