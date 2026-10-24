use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{platform, Done, Transfer, Verb};

const KEPT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undo {
    Transfer {
        back: Vec<Transfer>,
        restore: Vec<PathBuf>,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Remove(PathBuf),
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
        if done.landed.is_empty() {
            return;
        }
        let arrived: Vec<PathBuf> = done.landed.iter().map(|(_, to)| to.clone()).collect();
        let restore = done.replaced.clone();
        match done.verb {
            Verb::Copy => self.push(Undo::Trash {
                copies: arrived,
                restore,
            }),
            Verb::Move if done.landed.iter().all(|(from, to)| trashed(from, to)) => {
                self.push(Undo::Restore(arrived));
            }
            Verb::Move => self.push(Undo::Transfer {
                back: moves_back(&done.landed),
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

    pub fn undo(&mut self) -> Option<Undo> {
        let last = self.entries.pop()?;
        match last {
            Undo::Remove(made) if !empty_folder(&made) => None,
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

fn moves_back(landed: &[(PathBuf, PathBuf)]) -> Vec<Transfer> {
    let mut by_folder: BTreeMap<&Path, Vec<PathBuf>> = BTreeMap::new();
    for (from, to) in landed {
        let folder = from.parent().unwrap_or(Path::new(""));
        by_folder.entry(folder).or_default().push(to.clone());
    }
    by_folder
        .into_iter()
        .map(|(folder, sources)| Transfer {
            verb: Verb::Move,
            sources,
            into: folder.to_path_buf(),
        })
        .collect()
}

fn trashed(from: &Path, to: &Path) -> bool {
    from == to || platform::in_trash(to)
}

fn empty_folder(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}
