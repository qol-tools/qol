use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::copy::{copy_node, remove_all, same_content, same_link, Meter};
use super::simple::plain;
use super::{
    free_name, next_job_id, trash, Done, Failure, Job, JobId, Progress, Resolve, Survey, Verb,
};

const PULSE: Duration = Duration::from_millis(120);

pub fn start(survey: Survey, wake: impl Fn() + Send + 'static) -> Job {
    let id = next_job_id();
    let (tx, rx) = channel();
    thread::spawn(move || {
        let mut run = Run::new(id, &survey, tx, Box::new(wake));
        for source in &survey.transfer.sources {
            let copy_only = survey.copy_only.iter().any(|kept| kept.path == *source);
            run.item(source, &survey.transfer.into, copy_only);
        }
        run.finish();
    });
    Job { id, progress: rx }
}

enum Spot {
    Place(PathBuf),
    Merge,
    Skip,
}

struct Run {
    id: JobId,
    verb: Verb,
    choices: HashMap<PathBuf, Resolve>,
    tx: Sender<Progress>,
    wake: Box<dyn Fn() + Send>,
    total: u64,
    done: u64,
    last: Option<Instant>,
    placed: Vec<(PathBuf, PathBuf)>,
    landed: Vec<(PathBuf, PathBuf)>,
    replaced: Vec<PathBuf>,
    kept_originals: Vec<PathBuf>,
    failures: Vec<Failure>,
}

impl Meter for Run {
    fn advance(&mut self, bytes: u64) {
        self.done += bytes;
        if self.last.is_none_or(|at| at.elapsed() >= PULSE) {
            self.pulse();
        }
    }
}

impl Run {
    fn new(id: JobId, survey: &Survey, tx: Sender<Progress>, wake: Box<dyn Fn() + Send>) -> Self {
        Self {
            id,
            verb: survey.transfer.verb,
            choices: survey
                .clashes
                .iter()
                .map(|clash| (clash.existing.clone(), clash.choice))
                .collect(),
            tx,
            wake,
            total: survey.bytes,
            done: 0,
            last: None,
            placed: Vec::new(),
            landed: Vec::new(),
            replaced: Vec::new(),
            kept_originals: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn send(&self, progress: Progress) {
        let _ = self.tx.send(progress);
        (self.wake)();
    }

    fn pulse(&mut self) {
        self.last = Some(Instant::now());
        self.send(Progress::Bytes {
            done: self.done,
            total: self.total.max(self.done),
        });
    }

    fn fail(&mut self, path: &Path, error: &io::Error) {
        self.failures.push(Failure {
            path: path.to_path_buf(),
            reason: plain(error),
        });
    }

    fn finish(mut self) {
        self.done = self.total;
        self.pulse();
        let done = Done {
            id: self.id,
            verb: self.verb,
            placed: std::mem::take(&mut self.placed),
            landed: std::mem::take(&mut self.landed),
            replaced: std::mem::take(&mut self.replaced),
            kept_originals: std::mem::take(&mut self.kept_originals),
            failures: std::mem::take(&mut self.failures),
        };
        self.send(Progress::Finished(done));
    }

    fn item(&mut self, src: &Path, into: &Path, copy_only: bool) {
        let Some(name) = src.file_name() else {
            return;
        };
        let moving = self.verb == Verb::Move && !copy_only;
        let existing = into.join(name);
        let spot = if same_folder(src, into) {
            if self.verb == Verb::Move {
                return;
            }
            Spot::Place(free_name(into, name))
        } else {
            self.settle(src, &existing, into, name)
        };
        let landed = match spot {
            Spot::Skip => return,
            Spot::Place(target) => {
                self.send(Progress::Arriving(target.clone()));
                if !self.place(src, &target, moving) {
                    return;
                }
                if !copy_only {
                    self.landed.push((src.to_path_buf(), target.clone()));
                }
                target
            }
            Spot::Merge => {
                self.send(Progress::Arriving(existing.clone()));
                self.merge(src, &existing, moving);
                existing
            }
        };
        if copy_only {
            self.kept_originals.push(src.to_path_buf());
        } else {
            self.placed.push((src.to_path_buf(), landed));
        }
    }

    fn settle(&mut self, src: &Path, existing: &Path, parent: &Path, name: &OsStr) -> Spot {
        let Ok(here) = fs::symlink_metadata(existing) else {
            return Spot::Place(existing.to_path_buf());
        };
        let incoming_folder = fs::symlink_metadata(src).is_ok_and(|meta| meta.is_dir());
        if incoming_folder && here.is_dir() {
            return Spot::Merge;
        }
        match self.choices.get(existing).copied() {
            Some(Resolve::Replace) if self.discard(existing) => Spot::Place(existing.to_path_buf()),
            Some(Resolve::KeepBoth) => Spot::Place(free_name(parent, name)),
            _ => Spot::Skip,
        }
    }

    fn discard(&mut self, existing: &Path) -> bool {
        let trashed = trash(&[existing.to_path_buf()]);
        let clean = trashed.failures.is_empty();
        self.replaced
            .extend(trashed.placed.into_iter().map(|(_, location)| location));
        self.failures.extend(trashed.failures);
        clean
    }

    fn place(&mut self, src: &Path, target: &Path, moving: bool) -> bool {
        if moving {
            match fs::rename(src, target) {
                Ok(()) => return true,
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {}
                Err(error) => {
                    self.fail(src, &error);
                    return false;
                }
            }
        }
        let Some(partial) = partial_name(target) else {
            return false;
        };
        let _ = remove_all(&partial);
        if let Err(error) = copy_node(src, &partial, self) {
            let _ = remove_all(&partial);
            self.fail(src, &error);
            return false;
        }
        if let Err(error) = fs::rename(&partial, target) {
            let _ = remove_all(&partial);
            self.fail(src, &error);
            return false;
        }
        if moving {
            if let Err(error) = remove_all(src) {
                self.fail(src, &error);
            }
        }
        true
    }

    fn merge(&mut self, src: &Path, existing: &Path, moving: bool) -> bool {
        let mut children =
            match fs::read_dir(src).and_then(|dir| dir.collect::<io::Result<Vec<_>>>()) {
                Ok(children) => children,
                Err(error) => {
                    self.fail(src, &error);
                    return false;
                }
            };
        children.sort_by_key(|entry| entry.file_name());
        let mut emptied = true;
        for child in children {
            let path = child.path();
            let name = child.file_name();
            let target = existing.join(&name);
            if self.identical(&path, &target) {
                if moving {
                    if let Err(error) = fs::remove_file(&path) {
                        self.fail(&path, &error);
                        emptied = false;
                    }
                } else {
                    emptied = false;
                }
                continue;
            }
            emptied &= match self.settle(&path, &target, existing, &name) {
                Spot::Place(landing) => {
                    let placed = self.place(&path, &landing, moving);
                    if placed && (moving || self.verb == Verb::Copy) {
                        self.landed.push((path.clone(), landing));
                    }
                    placed && moving
                }
                Spot::Merge => self.merge(&path, &target, moving) && moving,
                Spot::Skip => false,
            };
        }
        if moving && emptied {
            if let Err(error) = fs::remove_dir(src) {
                self.fail(src, &error);
                return false;
            }
        }
        moving && emptied
    }

    fn identical(&self, src: &Path, target: &Path) -> bool {
        !self.choices.contains_key(target)
            && (same_link(src, target)
                || fs::symlink_metadata(target).is_ok_and(|meta| meta.is_file())
                    && fs::symlink_metadata(src).is_ok_and(|meta| meta.is_file())
                    && same_content(src, target))
    }
}

fn same_folder(src: &Path, into: &Path) -> bool {
    match (src.parent().map(fs::canonicalize), fs::canonicalize(into)) {
        (Some(Ok(parent)), Ok(into)) => parent == into,
        _ => false,
    }
}

fn partial_name(target: &Path) -> Option<PathBuf> {
    let name = target.file_name()?.to_string_lossy();
    Some(target.with_file_name(format!(".{name}.qol-partial")))
}
