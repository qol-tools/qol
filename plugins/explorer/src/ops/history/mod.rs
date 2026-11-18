use std::path::{Path, PathBuf};

use super::bin::Held;
use super::print::{Print, TreePrint};
use super::Verb;

#[cfg(test)]
mod faults;
mod inverse;
mod redo;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod walk;

pub use inverse::{start, Choices, Reply, Undoing};

const KEPT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StepId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Transfer {
        verb: Verb,
        sources: Vec<PathBuf>,
        into: PathBuf,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    NewFile(PathBuf),
    NewFolder(PathBuf),
    Trash(Vec<PathBuf>),
    Erase(Vec<PathBuf>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Removing,
    Removed,
    Partly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Moved {
        from: PathBuf,
        to: PathBuf,
        at: Print,
    },
    Copied {
        from: PathBuf,
        to: PathBuf,
        tree: TreePrint,
        original: TreePrint,
    },
    Crossed {
        from: PathBuf,
        to: PathBuf,
        tree: TreePrint,
        source: Source,
        from_device: Option<u64>,
    },
    Replaced {
        at: PathBuf,
        held: Option<Held>,
    },
    Matched {
        from: PathBuf,
        held: Option<Held>,
    },
    Emptied {
        folder: PathBuf,
        at: Print,
    },
    Made {
        path: PathBuf,
        at: Print,
    },
    Trashed {
        from: PathBuf,
        held: Option<Held>,
    },
    Erased {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undone {
    MovedBack {
        to: PathBuf,
        at: Print,
    },
    Binned(Option<Held>),
    Restored {
        to: PathBuf,
        tree: TreePrint,
    },
    CopiedBack {
        to: PathBuf,
        far: Option<Held>,
        kept: Option<String>,
        tree: TreePrint,
    },
    Removed,
    Remade {
        made: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Done,
    Undone(Box<Undone>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub change: Change,
    pub state: State,
    pub tried: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: StepId,
    pub action: Action,
    pub items: Vec<Item>,
    pub open: bool,
}

impl Step {
    pub fn waiting(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state == State::Done)
            .count()
    }

    pub fn undone(&self) -> bool {
        !self.items.is_empty() && self.waiting() == 0
    }

    pub fn pending(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.state, State::Undone(_)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Gone(PathBuf),
    Changed(PathBuf),
    OriginalGone(PathBuf),
    OriginalChanged(PathBuf),
    NotInTrash(PathBuf),
    Taken(PathBuf),
    Kept(PathBuf),
    Partly(PathBuf),
    CannotUndo(PathBuf),
    NoTrash(PathBuf, String),
    Unplugged(PathBuf),
    Failed(PathBuf, String),
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Undone(Undone),
    Redone(Change),
    Stays(Reason),
    Leaves(Reason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub index: usize,
    pub change: Change,
    pub retry: bool,
    pub undone: Option<Undone>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub step: StepId,
    pub items: Vec<Planned>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub step: StepId,
    pub outcomes: Vec<(usize, Outcome)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Top<'a> {
    Nothing,
    Empty(&'a Step),
    Erased(&'a Step),
    Undo(&'a Step),
}

#[derive(Debug, Default)]
pub struct History {
    steps: Vec<Step>,
    redo: Vec<StepId>,
    serial: u64,
}

impl History {
    pub fn open(&mut self, action: Action) -> StepId {
        self.steps.retain(|step| step.open || !step.undone());
        self.redo.clear();
        self.serial += 1;
        let id = StepId(self.serial);
        self.steps.push(Step {
            id,
            action,
            items: Vec::new(),
            open: true,
        });
        while self.steps.len() > KEPT {
            let Some(oldest) = self.steps.iter().position(|step| !step.open) else {
                break;
            };
            let removed = self.steps.remove(oldest);
            self.redo.retain(|kept| *kept != removed.id);
        }
        id
    }

    pub fn record(&mut self, step: StepId, change: Change) {
        if let Some(step) = self.step_mut(step) {
            step.items.push(Item {
                change,
                state: State::Done,
                tried: false,
            });
        }
    }

    pub fn amend(&mut self, step: StepId, landed: &Path, now: Source) {
        let Some(step) = self.step_mut(step) else {
            return;
        };
        for item in &mut step.items {
            if let Change::Crossed { to, source, .. } = &mut item.change {
                if to == landed {
                    *source = now;
                }
            }
        }
    }

    pub fn retract(&mut self, step: StepId, place: &Path) {
        if let Some(step) = self.step_mut(step) {
            step.items
                .retain(|item| !matches!(&item.change, Change::Replaced { at, .. } if at == place));
        }
    }

    pub fn close(&mut self, step: StepId) {
        let Some(step) = self.step_mut(step) else {
            return;
        };
        step.open = false;
        for item in &mut step.items {
            if let Change::Crossed { source, .. } = &mut item.change {
                if *source == Source::Removing {
                    *source = Source::Partly;
                }
            }
        }
    }

    pub fn top(&self) -> Top<'_> {
        for step in self.steps.iter().rev() {
            if step.open || step.undone() {
                continue;
            }
            if step.waiting() == 0 {
                return Top::Empty(step);
            }
            if matches!(step.action, Action::Erase(_)) {
                return Top::Erased(step);
            }
            return Top::Undo(step);
        }
        Top::Nothing
    }

    pub fn redo_top(&self) -> Option<&Step> {
        self.redo
            .iter()
            .rev()
            .filter_map(|id| self.step(*id))
            .find(|step| !step.open && step.pending())
    }

    pub fn drop_step(&mut self, step: StepId) {
        self.steps.retain(|kept| kept.id != step);
        self.redo.retain(|kept| *kept != step);
    }

    pub fn plan(&self, step: StepId, direction: Direction) -> Option<Plan> {
        let found = self.step(step)?;
        let planned = |index: usize, item: &Item, undone: Option<Undone>| Planned {
            index,
            change: item.change.clone(),
            retry: item.tried,
            undone,
        };
        let items: Vec<Planned> = match direction {
            Direction::Undo => found
                .items
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, item)| item.state == State::Done)
                .map(|(index, item)| planned(index, item, None))
                .collect(),
            Direction::Redo => found
                .items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| match &item.state {
                    State::Undone(undone) => Some(planned(index, item, Some((**undone).clone()))),
                    State::Done => None,
                })
                .collect(),
        };
        Some(Plan { step, items })
    }

    pub fn settle(&mut self, settled: &Settled) {
        let Some(step) = self.step_mut(settled.step) else {
            return;
        };
        let mut leaving = Vec::new();
        for (index, outcome) in &settled.outcomes {
            let Some(item) = step.items.get_mut(*index) else {
                continue;
            };
            match outcome {
                Outcome::Undone(undone) => {
                    item.state = State::Undone(Box::new(undone.clone()));
                    item.tried = false;
                }
                Outcome::Redone(change) => {
                    item.change = change.clone();
                    item.state = State::Done;
                    item.tried = false;
                }
                Outcome::Stays(Reason::Stopped) => {}
                Outcome::Stays(_) => item.tried = true,
                Outcome::Leaves(_) => leaving.push(*index),
            }
        }
        leaving.sort_unstable();
        leaving.dedup();
        for index in leaving.into_iter().rev() {
            step.items.remove(index);
        }
        if step.items.is_empty() {
            self.drop_step(settled.step);
            return;
        }
        let undid = settled
            .outcomes
            .iter()
            .any(|(_, outcome)| matches!(outcome, Outcome::Undone(_)));
        self.refresh_redo(settled.step, undid);
    }

    fn refresh_redo(&mut self, id: StepId, undid: bool) {
        let (fully, pending) = match self.step(id) {
            Some(step) => (step.undone(), step.pending()),
            None => return,
        };
        let listed = self.redo.contains(&id);
        if !pending {
            self.redo.retain(|kept| *kept != id);
            return;
        }
        if fully && (undid || !listed) {
            self.redo.retain(|kept| *kept != id);
            self.redo.push(id);
        }
    }

    pub fn step(&self, id: StepId) -> Option<&Step> {
        self.steps.iter().find(|step| step.id == id)
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn step_mut(&mut self, id: StepId) -> Option<&mut Step> {
        self.steps.iter_mut().find(|step| step.id == id)
    }
}
