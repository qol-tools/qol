use std::collections::HashMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread;

use super::redo;
use super::{Change, Direction, Outcome, Plan, Planned, Reason, Source, StepId, Undone};
use crate::ops::bin::{BinError, Held};
use crate::ops::copy::{copy_to, same_entry, Meter};
use crate::ops::print::{Kind, Print, TreePrint};
use crate::ops::simple::{free_name, move_noreplace, plain};
use crate::ops::transfer::stamp;
use crate::ops::{Clash, Resolve, Tools};

const CHANGED_DURING: &str = "it changed while it was being copied back";

pub type Choices = HashMap<usize, Resolve>;

#[derive(Debug, Clone)]
pub enum Reply {
    Asks(Vec<(usize, Clash)>),
    Item(usize, Outcome),
    Done,
}

pub struct Undoing {
    pub step: StepId,
    pub result: Receiver<Reply>,
    pub stop: Arc<AtomicBool>,
}

pub fn start(
    plan: Plan,
    direction: Direction,
    choices: Option<Choices>,
    tools: Tools,
    lock: Option<File>,
    wake: impl Fn() + Send + 'static,
) -> Undoing {
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let step = plan.step;
    thread::spawn(move || {
        let send = |reply: Reply| {
            let _ = tx.send(reply);
            wake();
        };
        let choices = match choices {
            Some(choices) => choices,
            None => {
                let clashes = match direction {
                    Direction::Undo => asks(&plan),
                    Direction::Redo => redo::asks(&plan),
                };
                if !clashes.is_empty() {
                    drop(lock);
                    send(Reply::Asks(clashes));
                    return;
                }
                Choices::new()
            }
        };
        for planned in &plan.items {
            let outcome = if stopped.load(Ordering::Relaxed) {
                Outcome::Stays(Reason::Stopped)
            } else {
                let work = Undo {
                    tools: &tools,
                    choice: choices.get(&planned.index).copied(),
                    retry: planned.retry,
                };
                match direction {
                    Direction::Undo => work.one(&planned.change),
                    Direction::Redo => work.again(&planned.change, planned.undone.as_ref()),
                }
            };
            send(Reply::Item(planned.index, outcome));
        }
        drop(lock);
        send(Reply::Done);
    });
    Undoing {
        step,
        result: rx,
        stop,
    }
}

fn asks(plan: &Plan) -> Vec<(usize, Clash)> {
    let mut vacated: Vec<&Path> = Vec::new();
    let mut clashes = Vec::new();
    for Planned { index, change, .. } in &plan.items {
        if let Some((home, current)) = destination(change) {
            let blocked = occupied(home)
                && occupied(current)
                && !vacated.contains(&home)
                && !same_entry(home, current);
            if blocked {
                if let Some(clash) = clash(home, current) {
                    clashes.push((*index, clash));
                }
            }
        }
        if let Some(left) = vacates(change) {
            vacated.push(left);
        }
    }
    clashes
}

fn destination(change: &Change) -> Option<(&Path, &Path)> {
    match change {
        Change::Moved { from, to, .. } => Some((from.as_path(), to.as_path())),
        Change::Crossed {
            from, to, source, ..
        } => (*source == Source::Removed).then_some((from.as_path(), to.as_path())),
        Change::Replaced { at, held } => held
            .as_ref()
            .map(|held| (at.as_path(), held.item.as_path())),
        Change::Matched { from, held } | Change::Trashed { from, held } => held
            .as_ref()
            .map(|held| (from.as_path(), held.item.as_path())),
        Change::Copied { .. }
        | Change::Emptied { .. }
        | Change::Made { .. }
        | Change::Erased { .. } => None,
    }
}

fn vacates(change: &Change) -> Option<&Path> {
    match change {
        Change::Moved { to, .. } | Change::Copied { to, .. } | Change::Crossed { to, .. } => {
            Some(to)
        }
        Change::Made { path, .. } => Some(path),
        Change::Replaced { .. }
        | Change::Matched { .. }
        | Change::Emptied { .. }
        | Change::Trashed { .. }
        | Change::Erased { .. } => None,
    }
}

pub(super) fn clash(home: &Path, current: &Path) -> Option<Clash> {
    let here = fs::symlink_metadata(home).ok()?;
    let incoming = fs::symlink_metadata(current).ok()?;
    Some(Clash {
        source: current.to_path_buf(),
        existing: home.to_path_buf(),
        here: stamp(home, &here),
        incoming: stamp(current, &incoming),
        same: false,
        choice: Resolve::KeepBoth,
    })
}

pub(super) enum Exit {
    Stays(Reason),
    Leaves(Reason),
}

type Tried = Result<Undone, Exit>;

pub(super) struct Undo<'a> {
    pub(super) tools: &'a Tools,
    pub(super) choice: Option<Resolve>,
    pub(super) retry: bool,
}

impl Undo<'_> {
    fn one(&self, change: &Change) -> Outcome {
        let tried = match change {
            Change::Moved { from, to, at } => self.move_back(from, to, at),
            Change::Copied {
                from,
                to,
                tree,
                original,
            } => self.bin_copy(from, to, tree, original),
            Change::Crossed {
                from,
                to,
                tree,
                source,
                from_device,
            } => self.carry_back(from, to, tree, *source, *from_device),
            Change::Replaced { at, held } => self.put_back(at, held.as_ref()),
            Change::Matched { from, held } | Change::Trashed { from, held } => {
                self.put_back(from, held.as_ref())
            }
            Change::Emptied { folder, at } => self.remake(folder, at),
            Change::Made { path, at } => self.unmake(path, at),
            Change::Erased { path } => Err(leaves(Reason::CannotUndo(path.clone()))),
        };
        match tried {
            Ok(undone) => Outcome::Undone(undone),
            Err(Exit::Stays(reason)) => Outcome::Stays(reason),
            Err(Exit::Leaves(reason)) => Outcome::Leaves(reason),
        }
    }

    fn move_back(&self, from: &Path, to: &Path, at: &Print) -> Tried {
        let now = self.present(to, at.device())?;
        if !at.same_object(&now) {
            return Err(leaves(Reason::Changed(to.to_path_buf())));
        }
        let home = self.landing(from, to)?;
        self.make_parent(&home, at.device())?;
        match move_noreplace(to, &home) {
            Ok(()) => {
                let landed = Print::read(&home).unwrap_or(*at);
                Ok(Undone::MovedBack {
                    to: home,
                    at: landed,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(leaves(Reason::Taken(home)))
            }
            Err(error) => Err(self.failed(to, plain(&error))),
        }
    }

    fn bin_copy(&self, from: &Path, to: &Path, tree: &TreePrint, original: &TreePrint) -> Tried {
        let now = self.tree_at(to, tree.root.device())?;
        if !tree.unchanged(&now) {
            return Err(leaves(Reason::Changed(to.to_path_buf())));
        }
        let source = match TreePrint::read(from, &self.tools.key) {
            Ok(source) => source,
            Err(error) if gone(&error) && !occupied(from) => {
                return Err(if unplugged(from, original.root.device()) {
                    stays(Reason::Unplugged(from.to_path_buf()))
                } else {
                    leaves(Reason::OriginalGone(from.to_path_buf()))
                });
            }
            Err(error) if gone(&error) => {
                return Err(leaves(Reason::OriginalChanged(from.to_path_buf())))
            }
            Err(error) => return Err(self.failed(from, plain(&error))),
        };
        if !original.unchanged(&source) {
            return Err(leaves(Reason::OriginalChanged(from.to_path_buf())));
        }
        self.bin(to).map(Undone::Binned)
    }

    fn carry_back(
        &self,
        from: &Path,
        to: &Path,
        tree: &TreePrint,
        source: Source,
        from_device: Option<u64>,
    ) -> Tried {
        if source != Source::Removed {
            return Err(leaves(Reason::Partly(to.to_path_buf())));
        }
        let now = self.tree_at(to, tree.root.device())?;
        if !tree.unchanged(&now) {
            return Err(leaves(Reason::Changed(to.to_path_buf())));
        }
        let home = self.landing(from, to)?;
        self.make_parent(&home, from_device)?;
        if let Err(error) = copy_to(to, &home, &mut Quiet, true) {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(leaves(Reason::Taken(home)));
            }
            return Err(self.failed(to, plain(&error)));
        }
        let copy_tree = TreePrint::read(&home, &self.tools.key).unwrap_or(*tree);
        let kept = match TreePrint::read(to, &self.tools.key) {
            Ok(after) if tree.unchanged(&after) => match self.tools.bin.trash(to) {
                Ok(far) => {
                    return Ok(Undone::CopiedBack {
                        to: home,
                        far,
                        kept: None,
                        tree: copy_tree,
                    })
                }
                Err(error) => error.reason(),
            },
            Ok(_) => CHANGED_DURING.into(),
            Err(error) => plain(&error),
        };
        Ok(Undone::CopiedBack {
            to: home,
            far: None,
            kept: Some(kept),
            tree: copy_tree,
        })
    }

    fn put_back(&self, home: &Path, held: Option<&Held>) -> Tried {
        let Some(held) = held else {
            return Err(leaves(Reason::CannotUndo(home.to_path_buf())));
        };
        let place = self.restore_held(home, held)?;
        let tree = TreePrint::read(&place, &self.tools.key).unwrap_or(TreePrint {
            root: held.before,
            entries: 0,
            bytes: 0,
            shape: 0,
            objects: 0,
        });
        Ok(Undone::Restored { to: place, tree })
    }

    pub(super) fn restore_held(&self, home: &Path, held: &Held) -> Result<PathBuf, Exit> {
        let place = self.landing(home, &held.item)?;
        self.make_parent(&place, held.before.device())?;
        match self.tools.bin.restore(held, &place) {
            Ok(()) => Ok(place),
            Err(BinError::Gone) if unplugged(&held.item, held.before.device()) => {
                Err(stays(Reason::Unplugged(home.to_path_buf())))
            }
            Err(BinError::Gone | BinError::Changed) => {
                Err(leaves(Reason::NotInTrash(home.to_path_buf())))
            }
            Err(BinError::Taken) => Err(leaves(Reason::Taken(place))),
            Err(BinError::NoTrash(reason)) => {
                Err(leaves(Reason::NoTrash(home.to_path_buf(), reason)))
            }
            Err(BinError::Failed(reason)) => Err(self.failed(home, reason)),
        }
    }

    fn remake(&self, folder: &Path, at: &Print) -> Tried {
        match fs::symlink_metadata(folder) {
            Ok(meta) if meta.is_dir() => return Ok(Undone::Remade { made: false }),
            Ok(_) => return Err(leaves(Reason::Taken(folder.to_path_buf()))),
            Err(error) if !gone(&error) => return Err(self.failed(folder, plain(&error))),
            Err(_) => {}
        }
        self.make_parent(folder, at.device())?;
        match fs::create_dir(folder) {
            Ok(()) => Ok(Undone::Remade { made: true }),
            Err(error) => Err(self.failed(folder, plain(&error))),
        }
    }

    fn unmake(&self, path: &Path, at: &Print) -> Tried {
        let now = self.present(path, at.device())?;
        if at.kind != Kind::Folder {
            if !at.unchanged(&now) {
                return Err(leaves(Reason::Changed(path.to_path_buf())));
            }
            return self.bin(path).map(Undone::Binned);
        }
        if !at.same_object(&now) {
            return Err(leaves(Reason::Changed(path.to_path_buf())));
        }
        match fs::remove_dir(path) {
            Ok(()) => Ok(Undone::Removed),
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                Err(leaves(Reason::Changed(path.to_path_buf())))
            }
            Err(error) => Err(self.failed(path, plain(&error))),
        }
    }

    pub(super) fn landing(&self, home: &Path, current: &Path) -> Result<PathBuf, Exit> {
        if !occupied(home) || same_entry(home, current) {
            return Ok(home.to_path_buf());
        }
        let taken = || home.to_path_buf();
        match self.choice {
            Some(Resolve::KeepBoth) => match (home.parent(), home.file_name()) {
                (Some(parent), Some(name)) => Ok(free_name(parent, name)),
                _ => Err(leaves(Reason::Taken(taken()))),
            },
            Some(Resolve::Keep | Resolve::Replace | Resolve::Merge) => {
                Err(leaves(Reason::Kept(taken())))
            }
            None => Err(leaves(Reason::Taken(taken()))),
        }
    }

    pub(super) fn make_parent(&self, place: &Path, device: Option<u64>) -> Result<(), Exit> {
        let Some(parent) = place.parent() else {
            return Ok(());
        };
        if unplugged(parent, device) {
            return Err(stays(Reason::Unplugged(place.to_path_buf())));
        }
        if fs::metadata(parent).is_ok() {
            return Ok(());
        }
        fs::create_dir_all(parent).map_err(|error| self.failed(place, plain(&error)))
    }

    pub(super) fn present(&self, path: &Path, device: Option<u64>) -> Result<Print, Exit> {
        match Print::read(path) {
            Ok(print) => Ok(print),
            Err(error) if gone(&error) => Err(missing(path, device)),
            Err(error) => Err(self.failed(path, plain(&error))),
        }
    }

    pub(super) fn tree_at(&self, path: &Path, device: Option<u64>) -> Result<TreePrint, Exit> {
        match TreePrint::read(path, &self.tools.key) {
            Ok(tree) => Ok(tree),
            Err(error) if gone(&error) && occupied(path) => {
                Err(leaves(Reason::Changed(path.to_path_buf())))
            }
            Err(error) if gone(&error) => Err(missing(path, device)),
            Err(error) => Err(self.failed(path, plain(&error))),
        }
    }

    pub(super) fn bin(&self, path: &Path) -> Result<Option<Held>, Exit> {
        self.tools.bin.trash(path).map_err(|error| match error {
            BinError::NoTrash(reason) => leaves(Reason::NoTrash(path.to_path_buf(), reason)),
            BinError::Gone => leaves(Reason::Gone(path.to_path_buf())),
            BinError::Changed | BinError::Taken | BinError::Failed(_) => {
                self.failed(path, error.reason())
            }
        })
    }

    pub(super) fn failed(&self, path: &Path, reason: String) -> Exit {
        let reason = Reason::Failed(path.to_path_buf(), reason);
        if self.retry {
            Exit::Leaves(reason)
        } else {
            Exit::Stays(reason)
        }
    }
}

pub(super) struct Quiet;

impl Meter for Quiet {
    fn advance(&mut self, _bytes: u64) {}
}

pub(super) fn missing(path: &Path, device: Option<u64>) -> Exit {
    if unplugged(path, device) {
        stays(Reason::Unplugged(path.to_path_buf()))
    } else {
        leaves(Reason::Gone(path.to_path_buf()))
    }
}

pub(super) fn unplugged(path: &Path, device: Option<u64>) -> bool {
    match (device, nearest_device(path)) {
        (Some(device), Some(near)) => device != near,
        _ => false,
    }
}

fn nearest_device(path: &Path) -> Option<u64> {
    path.ancestors().find_map(|ancestor| {
        let meta = fs::metadata(ancestor).ok()?;
        qol_fs::file_identity(ancestor, &meta).map(|found| found.device)
    })
}

pub(super) fn occupied(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub(super) fn gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

pub(super) fn leaves(reason: Reason) -> Exit {
    Exit::Leaves(reason)
}

pub(super) fn stays(reason: Reason) -> Exit {
    Exit::Stays(reason)
}
