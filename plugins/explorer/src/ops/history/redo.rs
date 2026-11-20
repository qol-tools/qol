use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::inverse::{clash, gone, leaves, missing, occupied, Exit, Quiet, Undo};
use super::{Change, Outcome, Plan, Planned, Reason, Source, Undone};
use crate::ops::bin::Held;
use crate::ops::copy::{copy_to, same_entry};
use crate::ops::print::{Print, TreePrint};
use crate::ops::simple::{move_noreplace, plain};
use crate::ops::Clash;

type Again = Result<Change, Exit>;

pub(super) fn asks(plan: &Plan) -> Vec<(usize, Clash)> {
    let mut vacated: Vec<&Path> = Vec::new();
    let mut clashes = Vec::new();
    for Planned {
        index,
        change,
        undone,
        ..
    } in &plan.items
    {
        let Some(undone) = undone else {
            continue;
        };
        if let Some((wanted, current)) = destination(change, undone) {
            let blocked = occupied(wanted)
                && occupied(current)
                && !vacated.contains(&wanted)
                && !same_entry(wanted, current);
            if blocked {
                if let Some(found) = clash(wanted, current) {
                    clashes.push((*index, found));
                }
            }
        }
        if let Some(left) = vacates(undone) {
            vacated.push(left);
        }
    }
    clashes
}

fn target(change: &Change) -> Option<&Path> {
    match change {
        Change::Moved { to, .. } | Change::Copied { to, .. } | Change::Crossed { to, .. } => {
            Some(to.as_path())
        }
        Change::Made { path, .. } => Some(path.as_path()),
        Change::Replaced { .. }
        | Change::Matched { .. }
        | Change::Emptied { .. }
        | Change::Trashed { .. }
        | Change::Erased { .. } => None,
    }
}

fn destination<'a>(change: &'a Change, undone: &'a Undone) -> Option<(&'a Path, &'a Path)> {
    let wanted = target(change)?;
    let current = match undone {
        Undone::MovedBack { to, .. } | Undone::CopiedBack { to, kept: None, .. } => to.as_path(),
        Undone::Binned(Some(held)) => held.item.as_path(),
        Undone::Binned(None)
        | Undone::CopiedBack { kept: Some(_), .. }
        | Undone::Restored { .. }
        | Undone::Removed
        | Undone::Remade { .. } => return None,
    };
    Some((wanted, current))
}

fn vacates(undone: &Undone) -> Option<&Path> {
    match undone {
        Undone::MovedBack { to, .. }
        | Undone::Restored { to, .. }
        | Undone::CopiedBack { to, kept: None, .. } => Some(to.as_path()),
        Undone::Binned(_)
        | Undone::CopiedBack { kept: Some(_), .. }
        | Undone::Removed
        | Undone::Remade { .. } => None,
    }
}

fn mismatch(path: &Path) -> Exit {
    leaves(Reason::Changed(path.to_path_buf()))
}

impl Undo<'_> {
    pub(super) fn again(&self, change: &Change, undone: Option<&Undone>) -> Outcome {
        let Some(undone) = undone else {
            return Outcome::Stays(Reason::Stopped);
        };
        let tried = match change {
            Change::Moved { to, .. } => self.move_forward(to, undone),
            Change::Copied {
                from,
                to,
                tree,
                original,
            } => self.copy_forward(from, to, tree, original, undone),
            Change::Crossed { to, tree, .. } => self.cross_forward(to, tree, undone),
            Change::Replaced { at, .. } => self
                .trash_forward(at, undone)
                .map(|(at, held)| Change::Replaced { at, held }),
            Change::Matched { from, .. } => self
                .trash_forward(from, undone)
                .map(|(from, held)| Change::Matched { from, held }),
            Change::Trashed { from, .. } => self
                .trash_forward(from, undone)
                .map(|(from, held)| Change::Trashed { from, held }),
            Change::Emptied { folder, at } => self.empty_forward(folder, at, undone),
            Change::Made { path, at } => self.make_forward(path, at, undone),
            Change::Erased { path } => Err(leaves(Reason::CannotUndo(path.clone()))),
        };
        match tried {
            Ok(done) => Outcome::Redone(done),
            Err(Exit::Stays(reason)) => Outcome::Stays(reason),
            Err(Exit::Leaves(reason)) => Outcome::Leaves(reason),
        }
    }

    fn move_forward(&self, wanted: &Path, undone: &Undone) -> Again {
        let Undone::MovedBack { to: home, at } = undone else {
            return Err(mismatch(wanted));
        };
        let now = self.present(home, at.device())?;
        if !at.same_object(&now) {
            return Err(leaves(Reason::Changed(home.clone())));
        }
        let landed = self.landing(wanted, home)?;
        self.make_parent(&landed, at.device())?;
        match move_noreplace(home, &landed) {
            Ok(()) => {
                let fresh = Print::read(&landed).unwrap_or(*at);
                Ok(Change::Moved {
                    from: home.clone(),
                    to: landed,
                    at: fresh,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(leaves(Reason::Taken(landed)))
            }
            Err(error) => Err(self.failed(home, plain(&error))),
        }
    }

    fn copy_forward(
        &self,
        from: &Path,
        wanted: &Path,
        tree: &TreePrint,
        original: &TreePrint,
        undone: &Undone,
    ) -> Again {
        let landed = self.restore_binned(wanted, undone)?;
        let fresh = TreePrint::read(&landed, &self.tools.key).unwrap_or(*tree);
        Ok(Change::Copied {
            from: from.to_path_buf(),
            to: landed,
            tree: fresh,
            original: *original,
        })
    }

    fn make_forward(&self, path: &Path, at: &Print, undone: &Undone) -> Again {
        if matches!(undone, Undone::Removed) {
            return self.make_folder(path, at);
        }
        let landed = self.restore_binned(path, undone)?;
        let fresh = Print::read(&landed).unwrap_or(*at);
        Ok(Change::Made {
            path: landed,
            at: fresh,
        })
    }

    fn make_folder(&self, path: &Path, at: &Print) -> Again {
        self.make_parent(path, at.device())?;
        match fs::create_dir(path) {
            Ok(()) => Ok(Change::Made {
                path: path.to_path_buf(),
                at: Print::read(path).unwrap_or(*at),
            }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(leaves(Reason::Taken(path.to_path_buf())))
            }
            Err(error) => Err(self.failed(path, plain(&error))),
        }
    }

    fn restore_binned(&self, wanted: &Path, undone: &Undone) -> Result<PathBuf, Exit> {
        let Undone::Binned(held) = undone else {
            return Err(mismatch(wanted));
        };
        let Some(held) = held else {
            return Err(leaves(Reason::NotInTrash(wanted.to_path_buf())));
        };
        self.restore_held(wanted, held)
    }

    fn trash_forward(&self, home: &Path, undone: &Undone) -> Result<(PathBuf, Option<Held>), Exit> {
        let Undone::Restored { to, tree } = undone else {
            return Err(mismatch(home));
        };
        let now = self.tree_at(to, tree.root.device())?;
        if !tree.unchanged(&now) {
            return Err(leaves(Reason::Changed(to.clone())));
        }
        let held = self.bin(to)?;
        Ok((to.clone(), held))
    }

    fn cross_forward(&self, wanted: &Path, far: &TreePrint, undone: &Undone) -> Again {
        let Undone::CopiedBack {
            to: home,
            tree,
            kept,
            ..
        } = undone
        else {
            return Err(mismatch(wanted));
        };
        if kept.is_some() {
            return Err(leaves(Reason::Partly(home.clone())));
        }
        let now = self.tree_at(home, tree.root.device())?;
        if !tree.unchanged(&now) {
            return Err(leaves(Reason::Changed(home.clone())));
        }
        let landed = self.landing(wanted, home)?;
        self.make_parent(&landed, far.root.device())?;
        if let Err(error) = copy_to(home, &landed, &mut Quiet, true) {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(leaves(Reason::Taken(landed)));
            }
            return Err(self.failed(home, plain(&error)));
        }
        let source = self.trash_source(home, tree);
        let fresh = TreePrint::read(&landed, &self.tools.key).unwrap_or(*far);
        Ok(Change::Crossed {
            from: home.clone(),
            to: landed,
            tree: fresh,
            source,
            from_device: tree.root.device(),
        })
    }

    fn trash_source(&self, home: &Path, tree: &TreePrint) -> Source {
        let untouched =
            TreePrint::read(home, &self.tools.key).is_ok_and(|after| tree.unchanged(&after));
        if untouched && self.tools.bin.trash(home).is_ok() {
            Source::Removed
        } else {
            Source::Partly
        }
    }

    fn empty_forward(&self, folder: &Path, at: &Print, undone: &Undone) -> Again {
        let Undone::Remade { made } = undone else {
            return Err(mismatch(folder));
        };
        let same = Change::Emptied {
            folder: folder.to_path_buf(),
            at: *at,
        };
        if !*made {
            return Ok(same);
        }
        match fs::remove_dir(folder) {
            Ok(()) => Ok(same),
            Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => {
                Err(leaves(Reason::Changed(folder.to_path_buf())))
            }
            Err(error) if gone(&error) => Err(missing(folder, at.device())),
            Err(error) => Err(self.failed(folder, plain(&error))),
        }
    }
}
