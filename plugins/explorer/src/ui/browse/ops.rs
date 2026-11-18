use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use super::ops_job::{Running, Trip};
use super::ops_text;
use super::Browser;
use crate::listing::Entry;
use crate::ops::history::{Action, History, Outcome, StepId, Undoing};
use crate::ops::{self, Failure, Resolve, Room, Survey, Tools, Transfer, Trashing, Verb};

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
    pub paths: BTreeSet<PathBuf>,
    pub picks_version: u64,
}

pub(super) enum Work {
    Surveyed(u64, io::Result<Survey>),
    Trashed(StepId, Trashing),
    Erased(StepId, Vec<PathBuf>, Vec<Failure>),
}

struct Waiting {
    survey: Survey,
    step: StepId,
    volume: String,
}

pub(super) struct UndoRun {
    pub undoing: Undoing,
    pub outcomes: Vec<(usize, Outcome)>,
    pub label: String,
}

pub(super) struct UndoAsk {
    pub step: StepId,
    pub indices: Vec<usize>,
    pub survey: Survey,
    pub lines: Vec<String>,
}

pub(super) struct Ops {
    pub holding: Option<Holding>,
    pub history: History,
    pub tools: Tools,
    pub records: Vec<MoveRecord>,
    pub moves_open: bool,
    pub notice: Option<String>,
    pub rename: Option<super::ops_edit::Renaming>,
    pub rename_after: Option<PathBuf>,
    pub erasing: Option<Vec<PathBuf>>,
    sender: Sender<Work>,
    work: Receiver<Work>,
    serial: u64,
    awaiting: Option<(u64, StepId)>,
    sheet: Option<Waiting>,
    pub running: Option<Running>,
    pub queued: VecDeque<(Survey, Trip)>,
    pub trashing: usize,
    pub deleting: usize,
    pub undo: Option<UndoRun>,
    pub asking: Option<UndoAsk>,
}

impl Ops {
    pub(super) fn new() -> Self {
        let (sender, work) = mpsc::channel();
        Self {
            holding: None,
            history: History::default(),
            tools: Tools::system(),
            records: Vec::new(),
            moves_open: false,
            notice: None,
            rename: None,
            rename_after: None,
            erasing: None,
            sender,
            work,
            serial: 0,
            awaiting: None,
            sheet: None,
            running: None,
            queued: VecDeque::new(),
            trashing: 0,
            deleting: 0,
            undo: None,
            asking: None,
        }
    }

    pub(super) fn paste_pending(&self) -> bool {
        self.awaiting.is_some() || self.sheet.is_some()
    }

    pub(super) fn busy_verb(&self) -> Option<Verb> {
        self.running
            .as_ref()
            .map(|running| running.trip.verb)
            .or_else(|| self.queued.front().map(|(_, trip)| trip.verb))
    }

    fn close_pending(&mut self) {
        if let Some((_, step)) = self.awaiting.take() {
            self.history.close(step);
        }
        if let Some(waiting) = self.sheet.take() {
            self.history.close(waiting.step);
        }
    }

    pub(super) fn sender(&self) -> Sender<Work> {
        self.sender.clone()
    }

    pub(super) fn leave_folder(&mut self) {
        self.notice = None;
        self.rename = None;
        self.rename_after = None;
        self.erasing = None;
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
        let paths: BTreeSet<PathBuf> = self.operands().into_iter().collect();
        if paths.is_empty() {
            return false;
        }
        self.ops.notice = None;
        self.failure = None;
        self.ops.holding = Some(Holding {
            held,
            paths,
            picks_version: self.picks.version(),
        });
        true
    }

    pub fn holds(&self) -> bool {
        self.ops.holding.is_some()
    }

    pub fn is_cut(&self, entry: &Entry) -> bool {
        self.ops
            .holding
            .as_ref()
            .is_some_and(|holding| holding.held == Held::Cut && holding.paths.contains(&entry.path))
    }

    pub fn paste(&mut self) -> Option<Transfer> {
        let into = self.paste_target();
        self.paste_into(into)
    }

    pub fn paste_here(&mut self) -> Option<Transfer> {
        let into = self.folder().to_path_buf();
        self.paste_into(into)
    }

    fn paste_into(&mut self, into: PathBuf) -> Option<Transfer> {
        let holding = self.ops.holding.as_ref()?;
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
            self.failure = Some(ops_text::already_in(&into, self.folder()));
            return None;
        }
        Some(Transfer {
            verb,
            sources,
            into,
        })
    }

    fn paste_target(&self) -> PathBuf {
        let here = self.folder();
        let Some(holding) = self.ops.holding.as_ref() else {
            return here.to_path_buf();
        };
        let held = |path: &Path| holding.paths.contains(path);
        let chosen = || {
            self.entry(self.cursor)
                .filter(|entry| self.chosen.as_deref() == Some(entry.path.as_path()))
        };
        let unchanged = self.picks.version() == holding.picks_version;
        let target = if unchanged || self.picks.count_in(here) == 0 {
            chosen()
        } else {
            let mut picked = self
                .picks
                .paths()
                .filter(|&path| path.parent() == Some(here) && !held(path));
            match (picked.next(), picked.next()) {
                (None, _) => chosen(),
                (Some(only), None) => self.entry_of(only),
                (Some(_), Some(_)) => None,
            }
        };
        target
            .filter(|entry| entry.opens_as_folder && !held(entry.path.as_path()))
            .map_or_else(|| here.to_path_buf(), |entry| entry.path.clone())
    }

    fn entry_of(&self, path: &Path) -> Option<&Entry> {
        self.entry(self.cursor)
            .filter(|entry| entry.path == path)
            .or_else(|| self.row_of(path).and_then(|row| self.entry(row)))
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

    pub fn duplicate_picks(&mut self) -> bool {
        if self.loading() || self.path.is_some() {
            return false;
        }
        let into = self.folder().to_path_buf();
        let sources: Vec<PathBuf> = self
            .operands()
            .into_iter()
            .filter(|path| path.parent() == Some(into.as_path()))
            .collect();
        if sources.is_empty() {
            return false;
        }
        self.ops.notice = None;
        self.begin(Transfer {
            verb: Verb::Duplicate,
            sources,
            into,
        });
        true
    }

    pub fn drop_hint(copy: bool) -> &'static str {
        ops_text::drop_hint(copy)
    }

    pub fn begin(&mut self, transfer: Transfer) {
        if transfer.sources.is_empty() {
            return;
        }
        self.ops.close_pending();
        self.ops.asking = None;
        let step = self.ops.history.open(Action::Transfer {
            verb: transfer.verb,
            sources: transfer.sources.clone(),
            into: transfer.into.clone(),
        });
        self.ops.serial += 1;
        let serial = self.ops.serial;
        self.ops.awaiting = Some((serial, step));
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
        self.ops
            .sheet
            .as_ref()
            .map(|waiting| &waiting.survey)
            .or_else(|| self.ops.asking.as_ref().map(|asking| &asking.survey))
    }

    pub fn undo_asking(&self) -> bool {
        self.ops.asking.is_some()
    }

    pub fn undo_lines(&self) -> Option<&[String]> {
        self.ops
            .asking
            .as_ref()
            .map(|asking| asking.lines.as_slice())
    }

    pub fn choose(&mut self, clash: usize, choice: Resolve) -> bool {
        if let Some(waiting) = self.ops.sheet.as_mut() {
            let Some(clash) = waiting.survey.clashes.get_mut(clash) else {
                return false;
            };
            clash.choice = clash.fitted(choice);
            return true;
        }
        let Some(asking) = self.ops.asking.as_mut() else {
            return false;
        };
        if !undo_choice(choice) {
            return false;
        }
        let Some(clash) = asking.survey.clashes.get_mut(clash) else {
            return false;
        };
        clash.choice = choice;
        true
    }

    pub fn choose_all(&mut self, choice: Resolve) -> bool {
        if let Some(waiting) = self.ops.sheet.as_mut() {
            for clash in &mut waiting.survey.clashes {
                clash.choice = clash.fitted(choice);
            }
            return true;
        }
        let Some(asking) = self.ops.asking.as_mut() else {
            return false;
        };
        if !undo_choice(choice) {
            return false;
        }
        for clash in &mut asking.survey.clashes {
            clash.choice = choice;
        }
        true
    }

    pub fn confirm(&mut self) -> bool {
        if self.ops.sheet.is_none() {
            let Some(asking) = self.ops.asking.take() else {
                return false;
            };
            let choices = asking
                .indices
                .iter()
                .copied()
                .zip(asking.survey.clashes.iter().map(|clash| clash.choice))
                .collect();
            self.start_undo(asking.step, Some(choices));
            return true;
        }
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
        self.enqueue(waiting.survey, waiting.step);
        true
    }

    pub fn cancel_survey(&mut self) -> bool {
        if self.ops.sheet.is_none() && self.ops.asking.take().is_some() {
            self.ops.notice = Some("Nothing was undone. ctrl+z asks again.".to_string());
            return true;
        }
        let waiting = self.ops.sheet.take();
        let awaited = self.ops.awaiting.take();
        if let Some((_, step)) = awaited {
            self.ops.history.close(step);
        }
        let Some(waiting) = waiting else {
            return awaited.is_some();
        };
        self.ops.history.close(waiting.step);
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

    pub fn selects(&self, row: usize) -> bool {
        if self.picks.is_empty() {
            return row == self.cursor && self.entry(row).is_some();
        }
        self.entry(row)
            .is_some_and(|entry| self.picks.is_picked(&entry.path))
    }

    pub fn operands(&self) -> Vec<PathBuf> {
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
        if let Some(run) = &self.ops.undo {
            return Some(ops_text::undoing_note(&run.label));
        }
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
        let into = self.paste_target();
        let target = (into.as_path() != self.folder()).then(|| ops_text::name_of(&into));
        Some(ops_text::held_note(
            holding.held,
            holding.paths.len(),
            target.as_deref(),
        ))
    }

    pub(super) fn drain_ops(&mut self) -> bool {
        let mut changed = false;
        while let Ok(work) = self.ops.work.try_recv() {
            self.apply_work(work);
            changed = true;
        }
        changed |= self.drain_job();
        changed |= self.drain_undo();
        changed |= self.open_pending_rename();
        changed
    }

    fn apply_work(&mut self, work: Work) {
        match work {
            Work::Surveyed(serial, result) => {
                let Some((awaited, step)) = self.ops.awaiting else {
                    return;
                };
                if awaited != serial {
                    return;
                }
                self.ops.awaiting = None;
                match result {
                    Ok(survey) => self.surveyed_in(survey, step),
                    Err(error) => {
                        self.ops.history.close(step);
                        self.failure = Some(format!("Could not read the files: {error}"));
                    }
                }
            }
            Work::Trashed(step, trashing) => self.trashed(step, trashing),
            Work::Erased(step, removed, failures) => self.erased(step, removed, failures),
        }
    }

    fn surveyed_in(&mut self, survey: Survey, step: StepId) {
        let needs_user = !survey.clashes.is_empty()
            || !survey.copy_only.is_empty()
            || matches!(survey.room, Room::Short { .. });
        if needs_user {
            let volume =
                crate::places::device_label(&crate::places::devices(), &survey.transfer.into)
                    .unwrap_or_else(|| "The destination".to_string());
            self.ops.sheet = Some(Waiting {
                survey,
                step,
                volume,
            });
            return;
        }
        self.enqueue(survey, step);
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

fn undo_choice(choice: Resolve) -> bool {
    choice == Resolve::Keep || choice == Resolve::KeepBoth
}
