use std::fs;
use std::path::{is_separator, Path, PathBuf, MAIN_SEPARATOR};

use futures::channel::mpsc::UnboundedSender;
use gpui::Keystroke;
use qol_gpui::crumb_trail::SUGGESTION_ROWS;
use qol_gpui::text_edit::marked;
use qol_gpui::{PathEdit, PathKey};

use super::watch::FolderWatch;
use crate::listing::{self, FailureReason, Listing, ListingError, ListingRead, ListingState, Pass};
use crate::nav::path::{self, PathInput};

pub(super) struct PathField {
    edit: PathEdit,
    prefill: String,
    folder: PathBuf,
    parent: PathBuf,
    segment: String,
    listing: Listing,
    incoming: Option<Listing>,
    read: Option<ListingRead>,
    watch: FolderWatch,
    wake: UnboundedSender<()>,
}

impl PathField {
    pub(super) fn open(folder: &Path, home: &Path, wake: UnboundedSender<()>) -> Self {
        let mut prefill = marked(&path::display_with_home(folder, home));
        if !prefill.ends_with(is_separator) {
            prefill.push(MAIN_SEPARATOR);
        }
        Self {
            edit: PathEdit::new(prefill.clone()),
            prefill,
            folder: folder.to_path_buf(),
            parent: PathBuf::new(),
            segment: String::new(),
            listing: Listing::new(PathBuf::new()),
            incoming: None,
            read: None,
            watch: FolderWatch::default(),
            wake,
        }
    }

    pub(super) fn edit(&self) -> &PathEdit {
        &self.edit
    }

    #[cfg(test)]
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

    pub(super) fn pick(&mut self, index: usize) -> bool {
        self.edit.pick(index)
    }

    pub(super) fn wheel(&mut self, rows: isize) -> bool {
        self.edit.wheel(rows, SUGGESTION_ROWS)
    }

    pub(super) fn target(&self, home: &Path) -> PathBuf {
        let text = self.edit.text();
        match text.strip_prefix(self.prefill.as_str()) {
            Some(rest) => path::collapse(&self.folder.join(rest.trim_start_matches(is_separator))),
            None => path::resolve(text, home, &self.folder),
        }
    }

    pub(super) fn refresh(&mut self, home: &Path, show_hidden: bool) {
        let (parent, segment) = self.typed_parent(home);
        self.segment = segment;
        if parent != self.parent {
            let _ = self.watch.arm(&parent, super::waker(&self.wake));
            self.read = Some(self.read_parent(&parent));
            self.listing = Listing::new(parent.clone());
            self.incoming = None;
            self.parent = parent;
        }
        self.refresh_suggestions(show_hidden);
    }

    pub(super) fn drain(&mut self, show_hidden: bool) -> bool {
        let events = super::drain_events(&mut self.read);
        let applied = !events.is_empty();
        let listing = self.incoming.as_mut().unwrap_or(&mut self.listing);
        for event in events {
            listing.apply(event);
        }
        if let Some(fresh) = self
            .incoming
            .take_if(|incoming| *incoming.state() != ListingState::Loading)
        {
            self.listing = fresh;
            let failed = matches!(self.listing.state(), ListingState::Failed(_));
            let _ = self
                .watch
                .rearm(&self.parent, failed, super::waker(&self.wake));
        }
        if self.read.is_none() && self.watch.changed() {
            self.read = Some(self.read_parent(&self.parent));
            self.incoming = Some(Listing::new(self.parent.clone()));
        }
        applied && self.refresh_suggestions(show_hidden)
    }

    fn typed_parent(&self, home: &Path) -> (PathBuf, String) {
        let text = self.edit.text();
        if let Some(rest) = text.strip_prefix(self.prefill.as_str()) {
            let rest = rest.trim_start_matches(is_separator);
            return match rest.rfind(is_separator) {
                Some(index) => (
                    path::collapse(&self.folder.join(&rest[..=index])),
                    rest[index + 1..].to_string(),
                ),
                None => (self.folder.clone(), rest.to_string()),
            };
        }
        let input = PathInput::parse(text, home);
        let parent = input.parent.map_or_else(
            || self.folder.clone(),
            |parent| path::collapse(&self.folder.join(parent)),
        );
        (parent, input.segment)
    }

    fn read_parent(&self, parent: &Path) -> ListingRead {
        listing::start(
            parent.to_path_buf(),
            Pass::NamesOnly,
            super::waker(&self.wake),
        )
    }

    pub(super) fn refresh_suggestions(&mut self, show_hidden: bool) -> bool {
        let names: Vec<String> = path::suggestions(&self.listing, &self.segment, show_hidden)
            .into_iter()
            .map(str::to_string)
            .collect();
        let changed = names != self.edit.suggestions();
        let selected = self.edit.selected();
        self.edit.set_suggestions(names);
        changed || self.edit.selected() != selected
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
