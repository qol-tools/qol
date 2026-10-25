use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use super::ops_job::{Running, Trip};
use super::ops_text;
use super::Browser;
use crate::listing::Entry;
use crate::ops::{self, Done, Failure, Journal, Resolve, Room, Survey, Transfer, Verb};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    Copied,
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveState {
    Running,
    Done,
    NotStarted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveRecord {
    pub title: String,
    pub detail: String,
    pub bytes: Option<u64>,
    pub state: MoveState,
}

pub(super) struct Holding {
    pub held: Held,
    pub paths: Vec<PathBuf>,
}

pub(super) enum Work {
    Surveyed(u64, io::Result<Survey>),
    Trashed(Done, bool),
    Restored(Vec<Failure>),
}

struct Waiting {
    survey: Survey,
    journaled: bool,
    volume: String,
}

pub(super) struct Ops {
    pub holding: Option<Holding>,
    pub journal: Journal,
    pub records: Vec<MoveRecord>,
    pub moves_open: bool,
    pub notice: Option<String>,
    pub rename: Option<super::ops_edit::Renaming>,
    pub rename_after: Option<PathBuf>,
    sender: Sender<Work>,
    work: Receiver<Work>,
    serial: u64,
    awaiting: Option<(u64, bool)>,
    sheet: Option<Waiting>,
    pub running: Option<Running>,
    pub queued: VecDeque<(Survey, Trip)>,
    pub undoing: Vec<Transfer>,
    pub restoring: Vec<PathBuf>,
    pub renaming: Vec<(PathBuf, PathBuf)>,
    pub returned: Vec<PathBuf>,
}

impl Ops {
    pub(super) fn stop_undo(&mut self) {
        self.undoing.clear();
        self.restoring.clear();
        self.renaming.clear();
        self.returned.clear();
    }

    pub(super) fn new() -> Self {
        let (sender, work) = mpsc::channel();
        Self {
            holding: None,
            journal: Journal::default(),
            records: Vec::new(),
            moves_open: false,
            notice: None,
            rename: None,
            rename_after: None,
            sender,
            work,
            serial: 0,
            awaiting: None,
            sheet: None,
            running: None,
            queued: VecDeque::new(),
            undoing: Vec::new(),
            restoring: Vec::new(),
            renaming: Vec::new(),
            returned: Vec::new(),
        }
    }

    pub(super) fn sender(&self) -> Sender<Work> {
        self.sender.clone()
    }

    pub(super) fn leave_folder(&mut self) {
        self.notice = None;
        self.rename = None;
        self.rename_after = None;
    }

    pub(super) fn record_at(&mut self, slot: usize) -> Option<&mut MoveRecord> {
        let index = self.records.len().checked_sub(1)?.checked_sub(slot)?;
        self.records.get_mut(index)
    }

    pub(super) fn open_record(&mut self, record: MoveRecord) -> usize {
        self.records.insert(0, record);
        self.records.len() - 1
    }
}

impl Browser {
    pub fn hold(&mut self, held: Held) -> bool {
        let paths = self.operands();
        if paths.is_empty() {
            return false;
        }
        self.ops.notice = None;
        self.failure = None;
        self.ops.holding = Some(Holding { held, paths });
        true
    }

    pub fn holds(&self) -> bool {
        self.ops.holding.is_some()
    }

    pub fn operand_count(&self) -> usize {
        self.operands().len()
    }

    pub fn is_cut(&self, entry: &Entry) -> bool {
        self.ops
            .holding
            .as_ref()
            .is_some_and(|holding| holding.held == Held::Cut && holding.paths.contains(&entry.path))
    }

    pub fn paste(&mut self) -> Option<Transfer> {
        let holding = self.ops.holding.as_ref()?;
        let into = self.folder().to_path_buf();
        let verb = match holding.held {
            Held::Copied => Verb::Copy,
            Held::Cut => Verb::Move,
        };
        let sources: Vec<PathBuf> = holding
            .paths
            .iter()
            .filter(|path| !into.starts_with(path))
            .filter(|path| verb == Verb::Copy || path.parent() != Some(into.as_path()))
            .cloned()
            .collect();
        if sources.is_empty() {
            self.failure = Some("Already in this folder".to_string());
            return None;
        }
        Some(Transfer {
            verb,
            sources,
            into,
        })
    }

    pub fn drop_on(&self, sources: Vec<PathBuf>, into: PathBuf, copy: bool) -> Transfer {
        let same_drive = !sources.iter().any(|source| ops::crosses(source, &into));
        let verb = if copy || !same_drive {
            Verb::Copy
        } else {
            Verb::Move
        };
        Transfer {
            verb,
            sources,
            into,
        }
    }

    pub fn drop_hint(copy: bool) -> &'static str {
        ops_text::drop_hint(copy)
    }

    pub fn begin(&mut self, transfer: Transfer) {
        self.begin_journaled(transfer, true);
    }

    pub(super) fn begin_journaled(&mut self, transfer: Transfer, journaled: bool) {
        if transfer.sources.is_empty() {
            return;
        }
        self.ops.serial += 1;
        let serial = self.ops.serial;
        self.ops.awaiting = Some((serial, journaled));
        self.ops.sheet = None;
        self.failure = None;
        let sender = self.ops.sender();
        let wake = self.waker();
        thread::spawn(move || {
            let result = ops::survey(transfer);
            let _ = sender.send(Work::Surveyed(serial, result));
            wake();
        });
    }

    pub fn surveyed(&self) -> Option<&Survey> {
        self.ops.sheet.as_ref().map(|waiting| &waiting.survey)
    }

    pub fn choose(&mut self, clash: usize, choice: Resolve) -> bool {
        let Some(clash) = self
            .ops
            .sheet
            .as_mut()
            .and_then(|waiting| waiting.survey.clashes.get_mut(clash))
        else {
            return false;
        };
        clash.choice = choice;
        true
    }

    pub fn choose_all(&mut self, choice: Resolve) -> bool {
        let Some(waiting) = self.ops.sheet.as_mut() else {
            return false;
        };
        for clash in &mut waiting.survey.clashes {
            clash.choice = choice;
        }
        true
    }

    pub fn confirm(&mut self) -> bool {
        let short = self
            .ops
            .sheet
            .as_ref()
            .is_none_or(|waiting| matches!(waiting.survey.room, Room::Short { .. }));
        if short {
            return false;
        }
        let Some(waiting) = self.ops.sheet.take() else {
            return false;
        };
        self.enqueue(waiting.survey, waiting.journaled);
        true
    }

    pub fn cancel_survey(&mut self) -> bool {
        let waiting = self.ops.sheet.take();
        let awaited = self.ops.awaiting.take().is_some();
        let Some(waiting) = waiting else {
            return awaited;
        };
        if !waiting.journaled {
            self.ops.stop_undo();
        }
        if matches!(waiting.survey.room, Room::Short { .. }) {
            let label = ops_text::label_of(&waiting.survey.transfer.sources);
            self.ops.open_record(MoveRecord {
                title: ops_text::title_of(&label, &waiting.survey.transfer.into),
                detail: ops_text::not_started_detail(&waiting.survey),
                bytes: None,
                state: MoveState::NotStarted,
            });
        }
        true
    }

    pub fn moves(&self) -> &[MoveRecord] {
        &self.ops.records
    }

    pub fn toggle_moves(&mut self) -> bool {
        self.ops.moves_open = !self.ops.moves_open;
        true
    }

    pub fn moves_open(&self) -> bool {
        self.ops.moves_open
    }

    pub fn arriving_bytes(&self, entry: &Entry) -> Option<(u64, u64)> {
        let running = self.ops.running.as_ref()?;
        let arriving = running.arriving.as_ref()?;
        (entry.path == *arriving || ops::partial_of(arriving, &entry.path))
            .then_some((running.done, running.total))
    }

    pub(super) fn operands(&self) -> Vec<PathBuf> {
        if !self.picks.is_empty() {
            let mut paths: Vec<PathBuf> = self.picks.paths().map(Path::to_path_buf).collect();
            paths.sort();
            return paths;
        }
        self.entry(self.cursor)
            .map(|entry| vec![entry.path.clone()])
            .unwrap_or_default()
    }

    pub(super) fn reload(&mut self) {
        self.changed_at.get_or_insert_with(Instant::now);
        (self.waker())();
    }

    pub(super) fn ops_note(&self) -> Option<String> {
        if let Some(running) = &self.ops.running {
            return Some(ops_text::running_progress(
                running.trip.verb,
                &running.trip.label,
                &running.trip.from_volume,
                running.done,
                running.total,
                running.started.elapsed(),
            ));
        }
        if let Some(waiting) = &self.ops.sheet {
            let note = ops_text::short_note(&waiting.survey, &waiting.volume)
                .or_else(|| ops_text::copy_only_note(&waiting.survey));
            if note.is_some() {
                return note;
            }
        }
        if let Some(notice) = &self.ops.notice {
            return Some(notice.clone());
        }
        let holding = self.ops.holding.as_ref()?;
        Some(ops_text::held_note(holding.held, holding.paths.len()))
    }

    pub(super) fn drain_ops(&mut self) -> bool {
        let mut changed = false;
        while let Ok(work) = self.ops.work.try_recv() {
            self.apply_work(work);
            changed = true;
        }
        changed |= self.drain_job();
        changed |= self.open_pending_rename();
        changed
    }

    fn apply_work(&mut self, work: Work) {
        match work {
            Work::Surveyed(serial, result) => {
                let Some((awaited, journaled)) = self.ops.awaiting else {
                    return;
                };
                if awaited != serial {
                    return;
                }
                self.ops.awaiting = None;
                match result {
                    Ok(survey) => self.surveyed_in(survey, journaled),
                    Err(error) => {
                        if !journaled {
                            self.ops.stop_undo();
                        }
                        self.failure = Some(format!("Could not read the files: {error}"));
                    }
                }
            }
            Work::Trashed(done, journaled) => self.trashed(done, journaled),
            Work::Restored(failures) => {
                self.failure = ops_text::failure_text(&failures);
                if failures.is_empty() {
                    self.ops.notice = Some("Put back where they were.".to_string());
                }
                self.reload();
            }
        }
    }

    fn surveyed_in(&mut self, survey: Survey, journaled: bool) {
        let needs_user = !survey.clashes.is_empty()
            || !survey.copy_only.is_empty()
            || matches!(survey.room, Room::Short { .. });
        if needs_user {
            let volume =
                crate::places::device_label(&crate::places::devices(), &survey.transfer.into)
                    .unwrap_or_else(|| "The destination".to_string());
            self.ops.sheet = Some(Waiting {
                survey,
                journaled,
                volume,
            });
            return;
        }
        self.enqueue(survey, journaled);
    }

    pub(super) fn dismiss_ops(&mut self) -> bool {
        let changed =
            self.ops.holding.is_some() || self.ops.notice.is_some() || self.ops.moves_open;
        self.ops.holding = None;
        self.ops.notice = None;
        self.ops.moves_open = false;
        changed
    }
}
