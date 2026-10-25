use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Done, Transfer, Verb};

const KEPT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undo {
    Transfer {
        back: Vec<Transfer>,
        renames: Vec<(PathBuf, PathBuf)>,
        restore: Vec<PathBuf>,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Remove(PathBuf),
    RemoveFile(PathBuf),
    Restore(Vec<PathBuf>),
    Trash {
        copies: Vec<PathBuf>,
        restore: Vec<PathBuf>,
    },
}

#[derive(Debug, Default)]
pub struct Journal {
    entries: Vec<Undo>,
}

impl Journal {
    pub fn record(&mut self, done: &Done) {
        if done.landed.is_empty() && done.matched.is_empty() {
            return;
        }
        let arrived: Vec<PathBuf> = done.landed.iter().map(|(_, to)| to.clone()).collect();
        let restore = done.replaced.clone();
        match done.verb {
            Verb::Copy | Verb::Duplicate => self.push(Undo::Trash {
                copies: arrived,
                restore,
            }),
            Verb::Move if done.trashed => self.push(Undo::Restore(arrived)),
            Verb::Move => self.push(Undo::Transfer {
                back: [
                    back_to_origin(&done.landed, Verb::Move),
                    back_to_origin(&done.matched, Verb::Copy),
                ]
                .concat(),
                renames: renamed(&done.landed),
                restore,
            }),
        }
    }

    pub fn record_rename(&mut self, from: PathBuf, to: PathBuf) {
        self.push(Undo::Rename { from: to, to: from });
    }

    pub fn record_new_folder(&mut self, made: PathBuf) {
        self.push(Undo::Remove(made));
    }

    pub fn record_new_file(&mut self, made: PathBuf) {
        self.push(Undo::RemoveFile(made));
    }

    pub fn undo(&mut self) -> Option<Undo> {
        let last = self.entries.pop()?;
        match last {
            Undo::Remove(made) if !empty_folder(&made) => None,
            Undo::RemoveFile(made) if !empty_file(&made) => None,
            other => Some(other),
        }
    }

    fn push(&mut self, undo: Undo) {
        self.entries.push(undo);
        if self.entries.len() > KEPT {
            self.entries.remove(0);
        }
    }
}

fn back_to_origin(landed: &[(PathBuf, PathBuf)], verb: Verb) -> Vec<Transfer> {
    let mut by_folder: BTreeMap<&Path, Vec<PathBuf>> = BTreeMap::new();
    for (from, to) in landed {
        let folder = from.parent().unwrap_or(Path::new(""));
        by_folder.entry(folder).or_default().push(to.clone());
    }
    by_folder
        .into_iter()
        .map(|(folder, sources)| Transfer {
            verb,
            sources,
            into: folder.to_path_buf(),
        })
        .collect()
}

fn renamed(landed: &[(PathBuf, PathBuf)]) -> Vec<(PathBuf, PathBuf)> {
    landed
        .iter()
        .filter(|(from, to)| from.file_name() != to.file_name())
        .filter_map(|(from, to)| Some((from.parent()?.join(to.file_name()?), from.clone())))
        .collect()
}

fn empty_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == 0)
}

fn empty_folder(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}
