use std::collections::hash_map::RandomState;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::SystemTime;

pub mod bin;
mod copy;
pub mod gate;
pub mod history;
mod platform;
pub mod print;
mod run;
mod simple;
mod transfer;

#[cfg(test)]
mod transfer_tests;

pub use bin::{Bin, BinError, FolderBin, Held};
pub use gate::Gate;
pub use history::{Change, Source};
pub use run::{partial_name, partial_of, start};
pub use simple::{copy_name, delete, free_name, new_file, new_folder, rename, trash, NameProblem};
pub use transfer::{crosses, survey};

#[derive(Clone)]
pub struct Tools {
    pub bin: Arc<dyn Bin>,
    pub key: RandomState,
    pub gate: Gate,
}

impl Tools {
    pub fn system() -> Self {
        Self {
            bin: bin::system(),
            key: RandomState::new(),
            gate: Gate::system(),
        }
    }

    pub fn at(root: &Path) -> Self {
        Self {
            bin: Arc::new(FolderBin::new(root.join("bin"))),
            key: RandomState::new(),
            gate: Gate::at(root.join("gate.lock")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Copy,
    Move,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub verb: Verb,
    pub sources: Vec<PathBuf>,
    pub into: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    pub folder: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    Keep,
    Replace,
    Merge,
    KeepBoth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clash {
    pub source: PathBuf,
    pub existing: PathBuf,
    pub here: Stamp,
    pub incoming: Stamp,
    pub same: bool,
    pub choice: Resolve,
}

impl Clash {
    pub fn merges(&self) -> bool {
        self.here.folder && self.incoming.folder
    }

    pub fn fitted(&self, choice: Resolve) -> Resolve {
        match choice {
            Resolve::Replace | Resolve::Merge if self.merges() => Resolve::Merge,
            Resolve::Merge => Resolve::Replace,
            other => other,
        }
    }

    pub fn applies(&self, clashes: &[Clash]) -> bool {
        !clashes.iter().any(|folder| {
            folder.merges()
                && folder.choice != Resolve::Merge
                && folder.existing != self.existing
                && self.existing.starts_with(&folder.existing)
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Room {
    Enough,
    Short { need: u64, free: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyOnly {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Survey {
    pub transfer: Transfer,
    pub clashes: Vec<Clash>,
    pub same_inside: usize,
    pub bytes: u64,
    pub room: Room,
    pub copy_only: Vec<CopyOnly>,
    pub crosses_volume: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(pub u64);

#[derive(Debug, Clone)]
pub enum Progress {
    Bytes { done: u64, total: u64 },
    Arriving(PathBuf),
    Placed(Box<Change>),
    Amended { to: PathBuf, source: Source },
    Retracted(PathBuf),
    Finished(Done),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Done {
    pub id: JobId,
    pub verb: Verb,
    pub placed: Vec<(PathBuf, PathBuf)>,
    pub kept_originals: Vec<PathBuf>,
    pub failures: Vec<Failure>,
}

#[derive(Debug, Clone)]
pub struct Trashing {
    pub trashed: Vec<(PathBuf, Option<Held>)>,
    pub failures: Vec<Failure>,
}

pub struct Job {
    pub id: JobId,
    pub progress: Receiver<Progress>,
}

static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_job_id() -> JobId {
    JobId(NEXT_JOB.fetch_add(1, Ordering::Relaxed))
}
