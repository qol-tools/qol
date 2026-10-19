mod counts;
mod path_field;
#[cfg(test)]
mod tests;
mod watch;

use std::collections::{HashMap, HashSet};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use gpui::Keystroke;
use qol_gpui::{PathEdit, PathKey};
use qol_watch::WatchError;

use super::keys::Step;
use crate::listing::{self, Entry, Listing, ListingEvent, ListingRead, ListingState, Pass};
use crate::nav::{History, Jump, Picks, Spot};
use counts::{LiveCounts, FOLLOWED_FOLDERS};
use path_field::PathField;
use watch::FolderWatch;

const REFRESH_QUIET: Duration = Duration::from_millis(150);
const STALLED_AFTER: Duration = Duration::from_secs(1);

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
    pub cursor_shift: isize,
    pub wake_in: Option<Duration>,
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
    progressed_at: Instant,
    landing: Option<PathBuf>,
    failure: Option<String>,
    path: Option<PathField>,
    incoming: Option<Listing>,
    counts: LiveCounts,
    changed_at: Option<Instant>,
    watch: FolderWatch,
    wake: UnboundedSender<()>,
    wakes: Option<UnboundedReceiver<()>>,
}

impl Browser {
    pub fn new(start: PathBuf, home: PathBuf) -> Self {
        let (wake, wakes) = mpsc::unbounded();
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
            progressed_at: Instant::now(),
            landing: None,
            failure: None,
            path: None,
            incoming: None,
            counts: LiveCounts::default(),
            changed_at: None,
            watch: FolderWatch::default(),
            wake,
            wakes: Some(wakes),
        };
        browser.arrive(Spot::at(start));
        browser
    }

    pub fn take_wakes(&mut self) -> Option<UnboundedReceiver<()>> {
        self.wakes.take()
    }

    pub fn waker(&self) -> impl Fn() + Send + Clone + 'static {
        waker(&self.wake)
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

    pub fn arriving(&self) -> bool {
        self.arriving.is_some()
    }

    pub fn stalled(&self, now: Instant) -> bool {
        self.arriving.is_some()
            && now.saturating_duration_since(self.progressed_at) >= STALLED_AFTER
    }

    #[cfg(test)]
    pub fn reading(&self) -> bool {
        self.read.is_some() || self.path.as_ref().is_some_and(PathField::reading)
    }

    #[cfg(test)]
    pub fn follows_counts(&self) -> bool {
        self.counts.following()
    }

    pub fn note(&self) -> Note {
        let count = if !self.picks.is_empty() {
            format!("{} picked", self.picks.len())
        } else if *self.listing.state() == ListingState::Done {
            qol_gpui::plural(self.rows.len(), "item", "items")
        } else {
            String::new()
        };
        Note {
            count,
            failure: self.failure.clone(),
        }
    }

    pub fn fail(&mut self, message: String) {
        self.failure = Some(message);
    }

    pub fn drain(&mut self, now: Instant) -> Drained {
        let events = drain_events(&mut self.read);
        if !events.is_empty() {
            self.progressed_at = now;
        }
        let mut drained = Drained::default();
        if self.incoming.is_some() {
            let before = self.cursor as isize;
            drained.changed = self.apply_incoming(events);
            drained.cursor_shift = self.cursor as isize - before;
            if self.read.is_none() {
                self.incoming = None;
            }
        } else if !events.is_empty() {
            for event in events {
                self.listing.apply(event);
            }
            self.refresh_rows();
            drained.changed = true;
            if let Some((scroll, cursor_shift)) = self.settle_arrival() {
                drained.scroll = Some(scroll);
                drained.cursor_shift = cursor_shift;
            }
        }
        drained.changed |= self.apply_recounts();
        let show_hidden = self.show_hidden;
        if let Some(field) = self.path.as_mut() {
            drained.changed |= field.drain(show_hidden);
        }
        let recount_in =
            self.counts
                .recount_when_due(now, &self.history.current().folder, waker(&self.wake));
        drained.wake_in = [self.refresh_when_due(now), recount_in]
            .into_iter()
            .flatten()
            .min();
        drained
    }

    pub fn go(&mut self, folder: PathBuf, scroll: f32) -> bool {
        if folder == self.folder() {
            return false;
        }
        let landing = crate::nav::path::child_toward(&folder, self.folder());
        self.remember(scroll);
        let spot = self.history.go(folder).clone();
        self.arrive(spot);
        self.landing = landing;
        true
    }

    pub fn up(&mut self, scroll: f32) -> bool {
        let Some(parent) = self.folder().parent().map(Path::to_path_buf) else {
            return false;
        };
        self.go(parent, scroll)
    }

    pub fn back(&mut self, scroll: f32) -> bool {
        self.remember(scroll);
        let Some(spot) = self.history.back().cloned() else {
            return false;
        };
        self.arrive(spot);
        true
    }

    pub fn forward(&mut self, scroll: f32) -> bool {
        self.remember(scroll);
        let Some(spot) = self.history.forward().cloned() else {
            return false;
        };
        self.arrive(spot);
        true
    }

    pub fn open(&mut self, scroll: f32) -> Opening {
        if self.loading() {
            return Opening::Nothing;
        }
        let Some(entry) = self.entry(self.cursor) else {
            return Opening::Nothing;
        };
        if !entry.opens_as_folder {
            let file = entry.path.clone();
            self.failure = None;
            return Opening::File(file);
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

    pub fn move_cursor(&mut self, step: Step, page: usize) -> bool {
        self.jump.reset();
        self.set_cursor(self.stepped(step, page))
    }

    pub fn extend_range(&mut self, step: Step, page: usize) -> bool {
        if self.loading() || self.rows.is_empty() {
            return false;
        }
        self.jump.reset();
        let to = self.stepped(step, page);
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

    pub fn dismiss(&mut self) -> bool {
        let changed = !self.picks.is_empty() || self.failure.is_some();
        self.picks.clear();
        self.failure = None;
        changed
    }

    pub fn toggle_hidden(&mut self) -> bool {
        let at = self.entry(self.cursor).map(|entry| entry.path.clone());
        self.picks.end_range();
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
        let mut field = PathField::open(self.folder(), &self.home, self.wake.clone());
        field.refresh(&self.home, self.show_hidden);
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
                field.refresh(&self.home, self.show_hidden);
                PathStep::Stay { changed: true }
            }
            PathKey::Handled => PathStep::Stay { changed: true },
            PathKey::Ignored => PathStep::Stay { changed: false },
            PathKey::Cancel => {
                self.path = None;
                self.failure = None;
                PathStep::Closed
            }
            PathKey::Submit => self.submit_path(scroll),
        }
    }

    pub fn pick_suggestion(&mut self, index: usize, scroll: f32) -> PathStep {
        let Some(field) = self.path.as_mut() else {
            return PathStep::Closed;
        };
        if !field.pick(index) {
            return PathStep::Stay { changed: false };
        }
        self.submit_path(scroll)
    }

    fn submit_path(&mut self, scroll: f32) -> PathStep {
        let Some(target) = self.path.as_ref().map(|field| field.target(&self.home)) else {
            return PathStep::Closed;
        };
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

    fn remember(&mut self, scroll: f32) {
        if self.arriving.is_none() {
            let entry = self.entry(self.cursor).map(|entry| entry.path.clone());
            self.history.remember(self.cursor, entry, scroll);
        }
    }

    fn loading(&self) -> bool {
        *self.listing.state() == ListingState::Loading
    }

    fn arrive(&mut self, spot: Spot) {
        self.incoming = None;
        self.counts = LiveCounts::default();
        self.changed_at = None;
        let watched = self.watch.arm(&spot.folder, waker(&self.wake));
        warn_unwatched(&spot.folder, watched);
        self.read = Some(listing::start(
            spot.folder.clone(),
            Pass::WithCounts,
            waker(&self.wake),
        ));
        self.listing = Listing::new(spot.folder.clone());
        self.rows.clear();
        self.cursor = 0;
        self.jump.reset();
        self.picks.end_range();
        self.failure = None;
        self.path = None;
        self.landing = None;
        self.arriving = Some(spot);
        self.progressed_at = Instant::now();
    }

    fn settle_arrival(&mut self) -> Option<(f32, isize)> {
        if self.loading() {
            return None;
        }
        let spot = self.arriving.take()?;
        let landing = self.landing.take();
        if matches!(self.listing.state(), ListingState::Failed(_)) {
            return Some((0.0, 0));
        }
        self.follow_counts();
        self.forget_vanished_picks();
        self.cursor = spot.cursor.min(self.rows.len().saturating_sub(1));
        if let Some(row) = landing.and_then(|child| self.row_of(&child)) {
            self.cursor = row;
            return Some((spot.scroll, 0));
        }
        let Some(row) = spot.entry.and_then(|entry| self.row_of(&entry)) else {
            return Some((spot.scroll, 0));
        };
        self.cursor = row;
        Some((spot.scroll, row as isize - spot.cursor as isize))
    }

    fn forget_vanished_picks(&mut self) {
        let folder = &self.history.current().folder;
        let present: HashSet<&Path> = self
            .listing
            .entries()
            .iter()
            .map(|entry| entry.path.as_path())
            .collect();
        self.picks
            .retain(|path| path.parent() != Some(folder.as_path()) || present.contains(path));
    }

    fn refresh_when_due(&mut self, now: Instant) -> Option<Duration> {
        if self.watch.changed() && self.changed_at.is_none() {
            self.changed_at = Some(now);
        }
        let since = self.changed_at?;
        if self.loading() || self.incoming.is_some() {
            return None;
        }
        let due = since + REFRESH_QUIET;
        if now < due {
            return Some(due - now);
        }
        self.changed_at = None;
        self.refresh();
        None
    }

    fn refresh(&mut self) {
        let folder = self.folder().to_path_buf();
        self.read = Some(listing::start(
            folder.clone(),
            Pass::WithCounts,
            waker(&self.wake),
        ));
        let mut incoming = Listing::new(folder);
        incoming.carry_counts(&self.listing);
        self.incoming = Some(incoming);
    }

    fn apply_incoming(&mut self, events: Vec<ListingEvent>) -> bool {
        let done = self.incoming.as_mut().is_some_and(|incoming| {
            for event in events {
                incoming.apply(event);
            }
            *incoming.state() != ListingState::Loading
        });
        if !done {
            return false;
        }
        let Some(fresh) = self.incoming.take() else {
            return false;
        };
        let at = self
            .entry(self.cursor)
            .map(|entry| (entry.path.clone(), entry.id));
        let before = mem::replace(&mut self.listing, fresh);
        self.refresh_rows();
        if let Some(row) = at.and_then(|(path, id)| self.row_following(&path, id, &before)) {
            self.cursor = row;
        }
        let failed = matches!(self.listing.state(), ListingState::Failed(_));
        if !failed {
            self.follow_renamed_picks(&before);
            self.forget_vanished_picks();
        }
        self.follow_counts();
        let folder = self.folder().to_path_buf();
        let watched = self.watch.rearm(&folder, failed, waker(&self.wake));
        warn_unwatched(&folder, watched);
        true
    }

    fn apply_recounts(&mut self) -> bool {
        let counts = self.counts.drain();
        if counts.is_empty() {
            return false;
        }
        if let Some(incoming) = self.incoming.as_mut() {
            incoming.apply(ListingEvent::Counts(counts.clone()));
        }
        self.listing.apply(ListingEvent::Counts(counts));
        true
    }

    fn follow_counts(&mut self) {
        let folders = self.listing.folder_paths(FOLLOWED_FOLDERS);
        self.counts
            .follow(folders, self.listing.started(), waker(&self.wake));
    }

    fn follow_renamed_picks(&mut self, before: &Listing) {
        if self.picks.is_empty() {
            return;
        }
        let listed: HashSet<&Path> = self
            .listing
            .entries()
            .iter()
            .map(|entry| entry.path.as_path())
            .collect();
        let mut gone: HashMap<(u64, u64), Vec<&Path>> = HashMap::new();
        for entry in before.entries() {
            if let Some(id) = entry.id.filter(|_| !listed.contains(entry.path.as_path())) {
                gone.entry(id).or_default().push(entry.path.as_path());
            }
        }
        gone.retain(|_, paths| paths.iter().any(|path| self.picks.is_picked(path)));
        if gone.is_empty() {
            return;
        }
        let known: HashSet<&Path> = before
            .entries()
            .iter()
            .map(|entry| entry.path.as_path())
            .collect();
        let mut appeared: HashMap<(u64, u64), Vec<PathBuf>> = HashMap::new();
        for entry in self.listing.entries() {
            if let Some(id) = entry.id.filter(|_| !known.contains(entry.path.as_path())) {
                appeared.entry(id).or_default().push(entry.path.clone());
            }
        }
        for (id, from) in gone {
            let to = appeared.remove(&id).unwrap_or_default();
            let picked: Vec<&Path> = from
                .into_iter()
                .filter(|path| self.picks.is_picked(path))
                .collect();
            for (from, to) in picked.into_iter().zip(to) {
                self.picks.rename(from, to);
            }
        }
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

    fn row_following(
        &self,
        path: &Path,
        id: Option<(u64, u64)>,
        before: &Listing,
    ) -> Option<usize> {
        self.row_of(path).or_else(|| {
            let id = id?;
            self.rows.iter().position(|&index| {
                let entry = &self.listing.entries()[index];
                entry.id == Some(id) && !before.entries().iter().any(|old| old.path == entry.path)
            })
        })
    }

    fn stepped(&self, step: Step, page: usize) -> usize {
        let last = self.rows.len().saturating_sub(1);
        match step {
            Step::Up => self.cursor.saturating_sub(1),
            Step::Down => (self.cursor + 1).min(last),
            Step::PageUp => self.cursor.saturating_sub(page),
            Step::PageDown => (self.cursor + page).min(last),
            Step::Home => 0,
            Step::End => last,
        }
    }
}

fn warn_unwatched(folder: &Path, watched: Result<(), WatchError>) {
    if let Err(error) = watched {
        log::warn!(
            "{} cannot be watched, watching its nearest parent instead: {error}",
            folder.display()
        );
    }
}

fn waker(wake: &UnboundedSender<()>) -> impl Fn() + Send + Clone + 'static {
    let wake = wake.clone();
    move || {
        let _ = wake.unbounded_send(());
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
