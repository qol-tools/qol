use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

mod copy;
mod journal;
mod platform;
mod run;
mod simple;
mod transfer;

#[cfg(test)]
mod transfer_tests;

pub use journal::{Journal, Undo};
pub use run::start;
pub use simple::{free_name, new_folder, rename, restore, trash, NameProblem};
pub use transfer::{same_volume, survey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Copy,
    Move,
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
    pub landed: Vec<(PathBuf, PathBuf)>,
    pub replaced: Vec<PathBuf>,
    pub kept_originals: Vec<PathBuf>,
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
