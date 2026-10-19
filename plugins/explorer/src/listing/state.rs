use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{Entry, ItemCount, ListingError, ListingEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingState {
    Loading,
    Done,
    Failed(ListingError),
}

#[derive(Debug)]
pub struct Listing {
    folder: PathBuf,
    state: ListingState,
    entries: Vec<Entry>,
    counts: HashMap<PathBuf, ItemCount>,
}

impl Listing {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            state: ListingState::Loading,
            entries: Vec::new(),
            counts: HashMap::new(),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn state(&self) -> &ListingState {
        &self.state
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn visible(&self, show_hidden: bool) -> Vec<&Entry> {
        self.visible_indices(show_hidden)
            .into_iter()
            .map(|index| &self.entries[index])
            .collect()
    }

    pub fn visible_indices(&self, show_hidden: bool) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| show_hidden || !entry.hidden)
            .map(|(index, _)| index)
            .collect()
    }

    pub fn shows_empty(&self, show_hidden: bool) -> bool {
        self.state == ListingState::Done
            && !self
                .entries
                .iter()
                .any(|entry| show_hidden || !entry.hidden)
    }

    pub fn item_count(&self, path: &Path) -> Option<ItemCount> {
        self.counts.get(path).copied()
    }

    pub fn carry_counts(&mut self, from: &Listing) {
        self.counts.extend(
            from.counts
                .iter()
                .map(|(path, count)| (path.clone(), *count)),
        );
    }

    pub fn apply(&mut self, event: ListingEvent) {
        match event {
            ListingEvent::Entries(batch) => {
                self.entries.extend(batch);
                self.entries.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
            }
            ListingEvent::Done => self.state = ListingState::Done,
            ListingEvent::Failed(error) => {
                self.entries.clear();
                self.state = ListingState::Failed(error);
            }
            ListingEvent::Counts(counts) => self
                .counts
                .extend(counts.into_iter().map(|count| (count.path, count.count))),
        }
    }
}
