use std::path::{Path, PathBuf};

use super::bin::Held;
use super::print::{Print, TreePrint};
use super::Verb;

#[cfg(test)]
mod faults;
mod inverse;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod walk;

pub use inverse::{start_undo, Choices, Reply, Undoing};

const KEPT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StepId(u64);

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
    },
    Binned(Option<Held>),
    Restored {
        to: PathBuf,
    },
    CopiedBack {
        to: PathBuf,
        far: Option<Held>,
        kept: Option<String>,
    },
    Removed,
    Remade {
        made: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Done,
    Undone(Undone),
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
    Stays(Reason),
    Leaves(Reason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub index: usize,
    pub change: Change,
    pub retry: bool,
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
    serial: u64,
}

impl History {
    pub fn open(&mut self, action: Action) -> StepId {
        self.steps.retain(|step| step.open || !step.undone());
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
            self.steps.remove(oldest);
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

    pub fn drop_step(&mut self, step: StepId) {
        self.steps.retain(|kept| kept.id != step);
    }

    pub fn plan(&self, step: StepId) -> Option<Plan> {
        let found = self.step(step)?;
        let items = found
            .items
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, item)| item.state == State::Done)
            .map(|(index, item)| Planned {
                index,
                change: item.change.clone(),
                retry: item.tried,
            })
            .collect();
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
                Outcome::Undone(undone) => item.state = State::Undone(undone.clone()),
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
