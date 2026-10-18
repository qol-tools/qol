mod path_field;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::Instant;

use gpui::Keystroke;
use qol_gpui::{PathEdit, PathKey};

use super::keys::Step;
use crate::listing::{self, Entry, Listing, ListingEvent, ListingRead, ListingState};
use crate::nav::{History, Jump, Picks, Spot};
use path_field::PathField;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    Nothing,
    Folder,
    File(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStep {
    Stay { changed: bool },
    Arrived,
    Closed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Drained {
    pub changed: bool,
    pub scroll: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub count: String,
    pub failure: Option<String>,
}

pub struct Browser {
    home: PathBuf,
    history: History,
    listing: Listing,
    read: Option<ListingRead>,
    rows: Vec<usize>,
    cursor: usize,
    picks: Picks,
    jump: Jump,
    show_hidden: bool,
    arriving: Option<Spot>,
    failure: Option<String>,
    path: Option<PathField>,
}

impl Browser {
    pub fn new(start: PathBuf, home: PathBuf) -> Self {
        let mut browser = Self {
            home,
            history: History::new(start.clone()),
            listing: Listing::new(start.clone()),
            read: None,
            rows: Vec::new(),
            cursor: 0,
            picks: Picks::new(),
            jump: Jump::new(),
            show_hidden: false,
            arriving: None,
            failure: None,
            path: None,
        };
        browser.arrive(Spot::at(start));
        browser
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn folder(&self) -> &Path {
        &self.history.current().folder
    }

    pub fn listing(&self) -> &Listing {
        &self.listing
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn entry(&self, row: usize) -> Option<&Entry> {
        self.rows
            .get(row)
            .map(|&index| &self.listing.entries()[index])
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn is_picked(&self, entry: &Entry) -> bool {
        self.picks.is_picked(&entry.path)
    }

    pub fn jump_run(&self, now: Instant) -> &str {
        self.jump.run(now)
    }

    pub fn path_edit(&self) -> Option<&PathEdit> {
        self.path.as_ref().map(PathField::edit)
    }

    pub fn path_open(&self) -> bool {
        self.path.is_some()
    }

    pub fn reading(&self) -> bool {
        self.read.is_some() || self.path.as_ref().is_some_and(PathField::reading)
    }

    pub fn note(&self) -> Note {
        let count = if self.picks.is_empty() {
            qol_gpui::plural(self.rows.len(), "item", "items")
        } else {
            format!("{} picked", self.picks.len())
        };
        Note {
            count,
            failure: self.failure.clone(),
        }
    }

    pub fn fail(&mut self, message: String) {
        self.failure = Some(message);
    }

    pub fn drain(&mut self) -> Drained {
        let events = drain_events(&mut self.read);
        let mut drained = Drained {
            changed: !events.is_empty(),
            scroll: None,
        };
        for event in events {
            self.listing.apply(event);
        }
        if drained.changed {
            self.refresh_rows();
            drained.scroll = self.settle_arrival();
        }
        let show_hidden = self.show_hidden;
        if let Some(field) = self.path.as_mut() {
            drained.changed |= field.drain(show_hidden);
        }
        drained
    }

    pub fn go(&mut self, folder: PathBuf, scroll: f32) -> bool {
        if folder == self.folder() {
            return false;
        }
        self.history.remember(self.cursor, scroll);
        let spot = self.history.go(folder).clone();
        self.arrive(spot);
        true
    }

    pub fn up(&mut self, scroll: f32) -> bool {
        self.history.remember(self.cursor, scroll);
        let Some(spot) = self.history.up().cloned() else {
            return false;
        };
        self.arrive(spot);
        true
    }

    pub fn back(&mut self, scroll: f32) -> bool {
        self.history.remember(self.cursor, scroll);
        let Some(spot) = self.history.back().cloned() else {
            return false;
        };
        self.arrive(spot);
        true
    }

    pub fn forward(&mut self, scroll: f32) -> bool {
        self.history.remember(self.cursor, scroll);
        let Some(spot) = self.history.forward().cloned() else {
            return false;
        };
        self.arrive(spot);
        true
    }

    pub fn open(&mut self, scroll: f32) -> Opening {
        let Some(entry) = self.entry(self.cursor) else {
            return Opening::Nothing;
        };
        if !entry.opens_as_folder {
            return Opening::File(entry.path.clone());
        }
        let folder = entry.path.clone();
        self.go(folder, scroll);
        Opening::Folder
    }

    pub fn set_cursor(&mut self, row: usize) -> bool {
        if row >= self.rows.len() || row == self.cursor {
            return false;
        }
        self.cursor = row;
        true
    }

    pub fn move_cursor(&mut self, step: Step) -> bool {
        self.set_cursor(self.stepped(step))
    }

    pub fn extend_range(&mut self, step: Step) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let to = self.stepped(step);
        let rows = self.listing.visible(self.show_hidden);
        self.picks.extend_range(&rows, self.cursor, to);
        self.cursor = to;
        true
    }

    pub fn end_range(&mut self) {
        self.picks.end_range();
    }

    pub fn toggle_pick(&mut self) -> bool {
        let Some(path) = self.entry(self.cursor).map(|entry| entry.path.clone()) else {
            return false;
        };
        self.picks.toggle(&path);
        true
    }

    pub fn add_pick(&mut self, row: usize) -> bool {
        let Some(path) = self.entry(row).map(|entry| entry.path.clone()) else {
            return false;
        };
        self.picks.add(&path);
        true
    }

    pub fn pick_all(&mut self) -> bool {
        let rows = self.listing.visible(self.show_hidden);
        self.picks.all(&rows);
        true
    }

    pub fn clear_picks(&mut self) -> bool {
        let had_picks = !self.picks.is_empty();
        self.picks.clear();
        had_picks
    }

    pub fn toggle_hidden(&mut self) -> bool {
        let at = self.entry(self.cursor).map(|entry| entry.path.clone());
        self.show_hidden = !self.show_hidden;
        self.refresh_rows();
        if let Some(row) = at.and_then(|path| self.row_of(&path)) {
            self.cursor = row;
        }
        if let Some(field) = self.path.as_mut() {
            field.refresh_suggestions(self.show_hidden);
        }
        true
    }

    pub fn jump(&mut self, typed: char, now: Instant) -> bool {
        let rows = self.listing.visible(self.show_hidden);
        let found = self
            .jump
            .type_char(typed, now, &rows, self.cursor, |entry| {
                entry.display.as_str()
            });
        if let Some(row) = found {
            self.cursor = row;
        }
        true
    }

    pub fn edit_path(&mut self) -> bool {
        if self.path.is_some() {
            return false;
        }
        let text = crate::nav::path::display_with_home(self.folder(), &self.home);
        let mut field = PathField::new(PathEdit::new(text));
        field.refresh(
            &self.home,
            self.history.current().folder.as_path(),
            self.show_hidden,
        );
        self.path = Some(field);
        true
    }

    pub fn path_key(
        &mut self,
        keystroke: &Keystroke,
        paste: impl FnOnce() -> Option<String>,
        scroll: f32,
    ) -> PathStep {
        let Some(field) = self.path.as_mut() else {
            return PathStep::Closed;
        };
        match field.key(keystroke, paste) {
            PathKey::Changed => {
                self.failure = None;
                field.refresh(&self.home, &self.history.current().folder, self.show_hidden);
                PathStep::Stay { changed: true }
            }
            PathKey::Handled => PathStep::Stay { changed: true },
            PathKey::Ignored => PathStep::Stay { changed: false },
            PathKey::Cancel => {
                self.path = None;
                PathStep::Closed
            }
            PathKey::Submit => self.submit_path(scroll),
        }
    }

    fn submit_path(&mut self, scroll: f32) -> PathStep {
        let Some(text) = self.path_edit().map(|edit| edit.text().to_string()) else {
            return PathStep::Closed;
        };
        let target = crate::nav::path::resolve(&text, &self.home, self.folder());
        if let Err(error) = path_field::folder_at(&target) {
            self.failure = Some(error.to_string());
            return PathStep::Stay { changed: true };
        }
        self.path = None;
        if self.go(target, scroll) {
            return PathStep::Arrived;
        }
        PathStep::Closed
    }

    fn arrive(&mut self, spot: Spot) {
        self.read = Some(listing::start(spot.folder.clone()));
        self.listing = Listing::new(spot.folder.clone());
        self.rows.clear();
        self.cursor = 0;
        self.jump.reset();
        self.picks.end_range();
        self.failure = None;
        self.arriving = Some(spot);
    }

    fn settle_arrival(&mut self) -> Option<f32> {
        if *self.listing.state() == ListingState::Loading {
            return None;
        }
        let spot = self.arriving.take()?;
        if matches!(self.listing.state(), ListingState::Failed(_)) {
            return Some(0.0);
        }
        self.cursor = spot.cursor.min(self.rows.len().saturating_sub(1));
        Some(spot.scroll)
    }

    fn refresh_rows(&mut self) {
        self.rows = self.listing.visible_indices(self.show_hidden);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    fn row_of(&self, path: &Path) -> Option<usize> {
        self.rows
            .iter()
            .position(|&index| self.listing.entries()[index].path == path)
    }

    fn stepped(&self, step: Step) -> usize {
        match step {
            Step::Up => self.cursor.saturating_sub(1),
            Step::Down => (self.cursor + 1).min(self.rows.len().saturating_sub(1)),
        }
    }
}

fn drain_events(read: &mut Option<ListingRead>) -> Vec<ListingEvent> {
    let mut events = Vec::new();
    let Some(active) = read.as_ref() else {
        return events;
    };
    loop {
        match active.events().try_recv() {
            Ok(event) => events.push(event),
            Err(TryRecvError::Empty) => return events,
            Err(TryRecvError::Disconnected) => break,
        }
    }
    *read = None;
    events
}
