use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::TryRecvError;
use std::thread;

use gpui::{Keystroke, Modifiers};
use qol_gpui::text_edit::{Clipboard, MemoryClipboard};
use qol_gpui::{PathEdit, PathKey};

use super::ops::{MoveRecord, MoveState, Stepping, UndoAsk, UndoRun, Work};
use super::ops_text;
use super::Browser;
use crate::listing::Entry;
use crate::ops::gate::Taken;
use crate::ops::history::{
    self, Action, Change, Choices, Direction, Outcome, Reply, Settled, StepId, Top, Undone,
};
use crate::ops::print::Print;
use crate::ops::{self, Failure, Room, Survey, Transfer, Trashing, Verb};

pub(super) struct Renaming {
    pub target: PathBuf,
    pub edit: PathEdit,
    original: String,
}

impl Renaming {
    fn open(entry: &Entry) -> Self {
        let name = entry.name.to_string_lossy().into_owned();
        let mut edit = PathEdit::new(name.clone());
        let stem = stem_chars(&name, entry.opens_as_folder);
        if stem == name.chars().count() {
            edit.select_all();
        } else {
            edit.place_caret(stem);
            edit.key(&select_to_start(), &MemoryClipboard::default());
        }
        Self {
            target: entry.path.clone(),
            edit,
            original: name,
        }
    }
}

fn select_to_start() -> Keystroke {
    Keystroke {
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::none()
        },
        key: "home".to_string(),
        key_char: None,
    }
}

fn stem_chars(name: &str, folder: bool) -> usize {
    if folder {
        return name.chars().count();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].chars().count(),
        _ => name.chars().count(),
    }
}

impl Browser {
    pub fn trash_picks(&mut self) -> bool {
        let paths = self.operands();
        if paths.is_empty() {
            return false;
        }
        self.ops.notice = None;
        self.failure = None;
        let step = self.ops.history.open(Action::Trash(paths.clone()));
        self.ops.trashing += 1;
        let sender = self.ops.sender();
        let wake = self.waker();
        let tools = self.ops.tools.clone();
        thread::spawn(move || {
            let trashing = {
                let _shared = tools.gate.share();
                ops::trash(&*tools.bin, &paths)
            };
            let _ = sender.send(Work::Trashed(step, trashing));
            wake();
        });
        true
    }

    pub(super) fn trashed(&mut self, step: StepId, trashing: Trashing) {
        self.ops.trashing = self.ops.trashing.saturating_sub(1);
        let undoable = trashing.trashed.iter().any(|(_, held)| held.is_some());
        let count = trashing.trashed.len();
        for (from, held) in trashing.trashed {
            self.ops
                .history
                .record(step, Change::Trashed { from, held });
        }
        self.ops.history.close(step);
        if count > 0 {
            self.ops.notice = Some(ops_text::trashed_note(count, undoable));
            self.picks.clear();
        }
        self.failure = ops_text::failure_text(&trashing.failures);
        self.reload();
    }

    pub fn erase_ask(&mut self) -> bool {
        if self.path.is_some() || self.loading() {
            return false;
        }
        let operands = self.operands();
        let held = operands.len();
        let paths: Vec<PathBuf> = operands
            .into_iter()
            .filter(|path| path.parent() == Some(self.folder()))
            .collect();
        if paths.is_empty() {
            if held > 0 {
                self.ops.notice = Some(ops_text::erase_elsewhere_note());
                return true;
            }
            return false;
        }
        self.ops.notice = None;
        self.failure = None;
        self.ops.erasing = Some(paths);
        true
    }

    pub fn erasing(&self) -> Option<&[PathBuf]> {
        self.ops.erasing.as_deref()
    }

    pub fn erase_cancel(&mut self) -> bool {
        let Some(paths) = self.ops.erasing.take() else {
            return false;
        };
        let step = self.ops.history.open(Action::Erase(paths));
        self.ops.history.close(step);
        true
    }

    pub fn erase_confirm(&mut self) -> bool {
        let Some(paths) = self.ops.erasing.take() else {
            return false;
        };
        let step = self.ops.history.open(Action::Erase(paths.clone()));
        self.ops.deleting += 1;
        let sender = self.ops.sender();
        let wake = self.waker();
        let tools = self.ops.tools.clone();
        thread::spawn(move || {
            let failures = {
                let _shared = tools.gate.share();
                ops::delete(&paths)
            };
            let removed: Vec<PathBuf> = paths
                .into_iter()
                .filter(|path| !failures.iter().any(|failure| failure.path == *path))
                .collect();
            let _ = sender.send(Work::Erased(step, removed, failures));
            wake();
        });
        true
    }

    pub(super) fn erased(&mut self, step: StepId, removed: Vec<PathBuf>, failures: Vec<Failure>) {
        self.ops.deleting = self.ops.deleting.saturating_sub(1);
        let count = removed.len();
        for path in removed {
            self.ops.history.record(step, Change::Erased { path });
        }
        self.ops.history.close(step);
        if count > 0 {
            self.picks.clear();
            self.ops.notice = Some(ops_text::erased_note(count));
        }
        self.failure = ops_text::failure_text(&failures);
        self.reload();
    }

    pub fn rename_open(&mut self) -> bool {
        if self.path.is_some() || self.ops.rename.is_some() {
            return false;
        }
        let Some(entry) = self.entry(self.cursor) else {
            return false;
        };
        let renaming = Renaming::open(entry);
        self.failure = None;
        self.ops.rename = Some(renaming);
        true
    }

    pub fn rename_field(&self) -> Option<&PathEdit> {
        self.ops.rename.as_ref().map(|renaming| &renaming.edit)
    }

    pub fn renaming(&self, entry: &Entry) -> bool {
        self.ops
            .rename
            .as_ref()
            .is_some_and(|renaming| renaming.target == entry.path)
    }

    pub fn rename_key(&mut self, keystroke: &Keystroke, clipboard: &dyn Clipboard) -> bool {
        let Some(renaming) = self.ops.rename.as_mut() else {
            return false;
        };
        match renaming.edit.key(keystroke, clipboard) {
            PathKey::Changed => {
                self.failure = None;
                true
            }
            PathKey::Handled | PathKey::Ignored => true,
            PathKey::Cancel => {
                self.ops.rename = None;
                self.failure = None;
                true
            }
            PathKey::Submit => {
                self.commit_rename();
                true
            }
        }
    }

    fn commit_rename(&mut self) {
        let Some(renaming) = self.ops.rename.as_ref() else {
            return;
        };
        let name = renaming.edit.text().to_string();
        let target = renaming.target.clone();
        if name == renaming.original {
            self.ops.rename = None;
            return;
        }
        let before = Print::read(&target).ok();
        match ops::rename(&target, &name) {
            Ok(renamed) => {
                let step = self.ops.history.open(Action::Rename {
                    from: target.clone(),
                    to: renamed.clone(),
                });
                let undoable = before.is_some();
                if let Some(at) = before {
                    self.ops.history.record(
                        step,
                        Change::Moved {
                            from: target.clone(),
                            to: renamed.clone(),
                            at,
                        },
                    );
                }
                self.ops.history.close(step);
                self.picks.rename(&target, renamed);
                self.ops.rename = None;
                self.failure = None;
                self.ops.notice = Some(if undoable {
                    format!("Renamed to {name}. ctrl+z undoes it.")
                } else {
                    format!("Renamed to {name}.")
                });
                self.reload();
            }
            Err(problem) => {
                self.failure = Some(ops_text::problem_text(&problem, &name));
            }
        }
    }

    pub fn new_folder(&mut self) -> bool {
        self.make_new("New folder", ops::new_folder, Action::NewFolder)
    }

    pub fn new_file(&mut self) -> bool {
        self.make_new("New file", ops::new_file, Action::NewFile)
    }

    fn make_new(
        &mut self,
        base: &str,
        make: fn(&Path, &str) -> Result<PathBuf, ops::NameProblem>,
        action: fn(PathBuf) -> Action,
    ) -> bool {
        if self.loading() || self.path.is_some() {
            return false;
        }
        self.ops.rename = None;
        let folder = self.folder().to_path_buf();
        let free = ops::free_name(&folder, OsStr::new(base));
        let name = ops_text::name_of(&free);
        match make(&folder, &name) {
            Ok(made) => {
                let at = Print::read(&made).ok();
                let step = self.ops.history.open(action(made.clone()));
                if let Some(at) = at {
                    self.ops.history.record(
                        step,
                        Change::Made {
                            path: made.clone(),
                            at,
                        },
                    );
                }
                self.ops.history.close(step);
                self.ops.rename_after = Some(made);
                self.failure = None;
                self.reload();
            }
            Err(problem) => {
                self.failure = Some(ops_text::problem_text(&problem, &name));
            }
        }
        true
    }

    pub(super) fn open_pending_rename(&mut self) -> bool {
        let Some(row) = self
            .ops
            .rename_after
            .as_deref()
            .and_then(|made| self.row_of(made))
        else {
            return false;
        };
        self.ops.rename_after = None;
        self.cursor = row;
        self.choose_cursor();
        self.rename_open()
    }

    pub fn undo(&mut self, keys_waiting: bool) -> bool {
        self.step_through(Direction::Undo, keys_waiting)
    }

    pub fn redo(&mut self, keys_waiting: bool) -> bool {
        self.step_through(Direction::Redo, keys_waiting)
    }

    pub fn stepping(&self) -> Stepping {
        let undo = match self.ops.history.top() {
            Top::Nothing => None,
            Top::Empty(step) | Top::Erased(step) | Top::Undo(step) => {
                Some(ops_text::menu_word(&step.action))
            }
        };
        let redo = self
            .ops
            .history
            .redo_top()
            .map(|step| ops_text::menu_word(&step.action));
        Stepping { undo, redo }
    }

    fn step_through(&mut self, direction: Direction, keys_waiting: bool) -> bool {
        if self.ops.asking.is_some() {
            return true;
        }
        self.ops.notice = None;
        self.failure = None;
        if let Some(when) = self.undo_busy(keys_waiting) {
            self.ops.notice = Some(ops_text::refused(direction, &when));
            return true;
        }
        match direction {
            Direction::Undo => self.undo_top(),
            Direction::Redo => self.redo_top(),
        }
    }

    fn undo_top(&mut self) -> bool {
        let here = self.folder().to_path_buf();
        let top = match self.ops.history.top() {
            Top::Nothing => None,
            Top::Empty(step) => Some((step.id, Some(ops_text::spent(&step.action, &here, false)))),
            Top::Erased(step) => Some((step.id, Some(ops_text::spent(&step.action, &here, true)))),
            Top::Undo(step) => Some((step.id, None)),
        };
        let Some((id, spent)) = top else {
            self.ops.notice = Some(ops_text::nothing_to(Direction::Undo));
            return false;
        };
        let Some(spent) = spent else {
            return self.start_step(id, Direction::Undo, None);
        };
        self.ops.history.drop_step(id);
        let next = ops_text::next_text(self.ops.history.top(), &here);
        let text = format!("{} {next}", spent.text);
        self.log_step(spent.title, text.clone(), spent.state);
        self.ops.notice = Some(text);
        true
    }

    fn redo_top(&mut self) -> bool {
        let Some(id) = self.ops.history.redo_top().map(|step| step.id) else {
            self.ops.notice = Some(ops_text::nothing_to(Direction::Redo));
            return false;
        };
        self.start_step(id, Direction::Redo, None)
    }

    fn undo_busy(&self, keys_waiting: bool) -> Option<String> {
        if let Some(run) = &self.ops.undo {
            return Some(format!("when the {} ends", ops_text::word(run.direction)));
        }
        if let Some(verb) = self.ops.busy_verb() {
            return Some(format!("when the {} ends", ops_text::verb_noun(verb)));
        }
        if self.ops.trashing > 0 {
            return Some("when the move to the trash ends".to_string());
        }
        if self.ops.deleting > 0 {
            return Some("when the delete ends".to_string());
        }
        if self.ops.paste_pending() {
            return Some("once the paste has started".to_string());
        }
        keys_waiting.then(|| "once the folder has loaded".to_string())
    }

    pub(super) fn start_step(
        &mut self,
        step: StepId,
        direction: Direction,
        choices: Option<Choices>,
    ) -> bool {
        let here = self.folder().to_path_buf();
        let label = self
            .ops
            .history
            .step(step)
            .map(|found| ops_text::step_label(&found.action, &here));
        let plan = self.ops.history.plan(step, direction);
        let (Some(label), Some(plan)) = (label, plan) else {
            self.ops.notice = Some(ops_text::nothing_to(direction));
            return false;
        };
        let lock = match self.ops.tools.gate.claim() {
            Ok(lock) => lock,
            Err(Taken) => {
                self.ops.notice = Some(ops_text::refused(
                    direction,
                    "when the move in another window ends",
                ));
                return true;
            }
        };
        let undoing = history::start(
            plan,
            direction,
            choices,
            self.ops.tools.clone(),
            lock,
            self.waker(),
        );
        self.ops.undo = Some(UndoRun {
            undoing,
            direction,
            outcomes: Vec::new(),
            label,
        });
        true
    }

    pub fn stop_undo(&mut self) -> bool {
        let Some(run) = self.ops.undo.as_ref() else {
            return false;
        };
        run.undoing.stop.store(true, Ordering::Relaxed);
        true
    }

    #[cfg(test)]
    pub fn undo_running(&self) -> bool {
        self.ops.undo.is_some()
    }

    pub(super) fn drain_undo(&mut self) -> bool {
        let Some(run) = self.ops.undo.as_mut() else {
            return false;
        };
        let mut changed = false;
        let mut asked = None;
        let mut ended = None;
        loop {
            match run.undoing.result.try_recv() {
                Ok(Reply::Item(index, outcome)) => {
                    run.outcomes.push((index, outcome));
                    changed = true;
                }
                Ok(Reply::Asks(clashes)) => {
                    asked = Some(clashes);
                    break;
                }
                Ok(Reply::Done) => {
                    ended = Some(false);
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    ended = Some(true);
                    break;
                }
            }
        }
        if let Some(clashes) = asked {
            if let Some(run) = self.ops.undo.take() {
                self.open_undo_sheet(run.undoing.step, run.direction, clashes);
            }
            return true;
        }
        if let Some(vanished) = ended {
            self.finish_undo(vanished);
            return true;
        }
        changed
    }

    fn open_undo_sheet(
        &mut self,
        step: StepId,
        direction: Direction,
        asked: Vec<(usize, ops::Clash)>,
    ) {
        let (indices, clashes): (Vec<usize>, Vec<ops::Clash>) = asked.into_iter().unzip();
        let here = self.folder().to_path_buf();
        let into = clashes
            .first()
            .and_then(|clash| clash.existing.parent())
            .map_or_else(|| here.clone(), Path::to_path_buf);
        let lines = ops_text::undo_sheet_lines(direction, &clashes, &here);
        let survey = Survey {
            transfer: Transfer {
                verb: Verb::Move,
                sources: clashes.iter().map(|clash| clash.source.clone()).collect(),
                into,
            },
            clashes,
            same_inside: 0,
            bytes: 0,
            room: Room::Enough,
            copy_only: Vec::new(),
            crosses_volume: false,
        };
        self.ops.asking = Some(UndoAsk {
            step,
            direction,
            indices,
            survey,
            lines,
        });
    }

    fn finish_undo(&mut self, vanished: bool) {
        let Some(run) = self.ops.undo.take() else {
            return;
        };
        let id = run.undoing.step;
        let direction = run.direction;
        let outcomes = run.outcomes;
        let step = self.ops.history.step(id).cloned();
        self.ops.history.settle(&Settled {
            step: id,
            outcomes: outcomes.clone(),
        });
        let here = self.folder().to_path_buf();
        if let Some(step) = step {
            let (next, reversible) = self.follow_up(id, direction, &here);
            let text = ops_text::report_note(
                &step,
                &outcomes,
                &ops_text::Report {
                    direction,
                    stopped: vanished,
                    here: &here,
                    next: &next,
                    reversible,
                },
            );
            let did = outcomes
                .iter()
                .any(|(_, outcome)| matches!(outcome, Outcome::Undone(_) | Outcome::Redone(_)));
            let title = ops_text::step_title(direction, &step.action, did);
            let state = if did {
                MoveState::Done
            } else {
                MoveState::Failed
            };
            self.log_step(title, text.clone(), state);
            self.ops.notice = Some(text);
            self.rename_picks(&step, &outcomes);
        }
        self.reload();
    }

    fn follow_up(&self, id: StepId, direction: Direction, here: &Path) -> (String, bool) {
        let steps = &self.ops.history;
        match direction {
            Direction::Undo => (
                ops_text::next_text(steps.top(), here),
                steps.redo_top().is_some_and(|step| step.id == id),
            ),
            Direction::Redo => (
                ops_text::next_redo_text(steps.redo_top(), here),
                matches!(steps.top(), Top::Undo(step) if step.id == id),
            ),
        }
    }

    fn log_step(&mut self, title: String, detail: String, state: MoveState) {
        self.ops.open_record(MoveRecord {
            title,
            detail,
            bytes: None,
            state,
        });
    }

    fn rename_picks(&mut self, step: &history::Step, outcomes: &[(usize, Outcome)]) {
        let Action::Rename { to, .. } = &step.action else {
            return;
        };
        for (_, outcome) in outcomes {
            if let Outcome::Undone(Undone::MovedBack { to: home, .. }) = outcome {
                self.picks.rename(to, home.clone());
            } else if let Outcome::Redone(Change::Moved {
                from, to: renamed, ..
            }) = outcome
            {
                self.picks.rename(from, renamed.clone());
            }
        }
    }
}
