use std::fs;
use std::path::{Path, PathBuf};

use gpui::Keystroke;
use qol_gpui::{PathEdit, PathKey};

use crate::listing::{self, FailureReason, Listing, ListingError, ListingRead};
use crate::nav::path::{self, PathInput};

pub(super) struct PathField {
    edit: PathEdit,
    parent: PathBuf,
    segment: String,
    listing: Listing,
    read: Option<ListingRead>,
}

impl PathField {
    pub(super) fn new(edit: PathEdit) -> Self {
        Self {
            edit,
            parent: PathBuf::new(),
            segment: String::new(),
            listing: Listing::new(PathBuf::new()),
            read: None,
        }
    }

    pub(super) fn edit(&self) -> &PathEdit {
        &self.edit
    }

    pub(super) fn reading(&self) -> bool {
        self.read.is_some()
    }

    pub(super) fn key(
        &mut self,
        keystroke: &Keystroke,
        paste: impl FnOnce() -> Option<String>,
    ) -> PathKey {
        self.edit.key(keystroke, paste)
    }

    pub(super) fn refresh(&mut self, home: &Path, folder: &Path, show_hidden: bool) {
        let input = PathInput::parse(self.edit.text(), home);
        let parent = input
            .parent
            .map_or_else(|| folder.to_path_buf(), |parent| folder.join(parent));
        self.segment = input.segment;
        if parent != self.parent {
            self.read = Some(listing::start(parent.clone()));
            self.listing = Listing::new(parent.clone());
            self.parent = parent;
        }
        self.refresh_suggestions(show_hidden);
    }

    pub(super) fn drain(&mut self, show_hidden: bool) -> bool {
        let events = super::drain_events(&mut self.read);
        if events.is_empty() {
            return false;
        }
        for event in events {
            self.listing.apply(event);
        }
        self.refresh_suggestions(show_hidden)
    }

    pub(super) fn refresh_suggestions(&mut self, show_hidden: bool) -> bool {
        let names: Vec<String> = path::suggestions(&self.listing, &self.segment, show_hidden)
            .into_iter()
            .map(str::to_string)
            .collect();
        if names == self.edit.suggestions() {
            return false;
        }
        self.edit.set_suggestions(names);
        true
    }
}

pub(super) fn folder_at(target: &Path) -> Result<(), ListingError> {
    let metadata = fs::metadata(target)
        .map_err(|error| ListingError::from_io(target.to_path_buf(), &error))?;
    if metadata.is_dir() {
        return Ok(());
    }
    Err(ListingError {
        folder: target.to_path_buf(),
        reason: FailureReason::NotAFolder,
    })
}
