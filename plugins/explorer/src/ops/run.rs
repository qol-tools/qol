use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::copy::{
    copy_to, remove_copied, same_content, same_entry, same_link, same_object, Meter, ALREADY_THERE,
};
use super::print::{Print, TreePrint};
use super::simple::plain;
use super::{
    copy_name, free_name, next_job_id, Bin, Change, Done, Failure, Held, Job, JobId, Progress,
    Resolve, Source, Survey, Tools, Verb,
};

const PULSE: Duration = Duration::from_millis(120);
const PARTIAL_SUFFIX: &str = ".qol-partial";
const PARTIAL_NAME_BYTES: usize = 230;

pub fn start(survey: Survey, tools: Tools, wake: impl Fn() + Send + 'static) -> Job {
    let id = next_job_id();
    let (tx, rx) = channel();
    thread::spawn(move || {
        let shared = tools.gate.share();
        let mut run = Run::new(id, &survey, tools, tx, Box::new(wake));
        for source in &survey.transfer.sources {
            let copy_only = survey.copy_only.iter().any(|kept| kept.path == *source);
            run.item(source, &survey.transfer.into, copy_only);
        }
        run.finish(shared);
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
    bin: Arc<dyn Bin>,
    key: RandomState,
    tx: Sender<Progress>,
    wake: Box<dyn Fn() + Send>,
    total: u64,
    done: u64,
    last: Option<Instant>,
    placed: Vec<(PathBuf, PathBuf)>,
    replaced: Vec<(PathBuf, Option<Held>)>,
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
    fn new(
        id: JobId,
        survey: &Survey,
        tools: Tools,
        tx: Sender<Progress>,
        wake: Box<dyn Fn() + Send>,
    ) -> Self {
        Self {
            id,
            verb: survey.transfer.verb,
            choices: survey
                .clashes
                .iter()
                .map(|clash| (clash.existing.clone(), clash.choice))
                .collect(),
            bin: tools.bin,
            key: tools.key,
            tx,
            wake,
            total: survey.bytes,
            done: 0,
            last: None,
            placed: Vec::new(),
            replaced: Vec::new(),
            kept_originals: Vec::new(),
            failures: Vec::new(),
        }
    }

    fn send(&self, progress: Progress) {
        let _ = self.tx.send(progress);
        (self.wake)();
    }

    fn placed(&self, change: Change) {
        self.send(Progress::Placed(Box::new(change)));
    }

    fn pulse(&mut self) {
        self.last = Some(Instant::now());
        self.send(Progress::Bytes {
            done: self.done,
            total: self.total.max(self.done),
        });
    }

    fn fail(&mut self, path: &Path, error: &io::Error) {
        self.report(path, plain(error));
    }

    fn report(&mut self, path: &Path, reason: String) {
        self.failures.push(Failure {
            path: path.to_path_buf(),
            reason,
        });
    }

    fn finish(mut self, shared: Option<File>) {
        self.done = self.total;
        self.pulse();
        let done = Done {
            id: self.id,
            verb: self.verb,
            placed: std::mem::take(&mut self.placed),
            kept_originals: std::mem::take(&mut self.kept_originals),
            failures: std::mem::take(&mut self.failures),
        };
        drop(shared);
        self.send(Progress::Finished(done));
    }

    fn item(&mut self, src: &Path, into: &Path, copy_only: bool) {
        let Some(name) = src.file_name() else {
            return;
        };
        let moving = self.verb == Verb::Move && !copy_only;
        let existing = into.join(name);
        let before = self.replaced.len();
        let spot = if same_folder(src, into) {
            if self.verb == Verb::Move {
                return;
            }
            if self.verb == Verb::Duplicate {
                let folder = fs::symlink_metadata(src).is_ok_and(|meta| meta.is_dir());
                Spot::Place(copy_name(into, name, folder))
            } else {
                Spot::Place(free_name(into, name))
            }
        } else {
            self.settle(src, &existing, into, name)
        };
        let landed = match spot {
            Spot::Skip => return,
            Spot::Place(target) => {
                self.send(Progress::Arriving(target.clone()));
                if !self.place(src, &target, moving) {
                    self.put_back(before);
                    return;
                }
                self.replaced.truncate(before);
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
            return match self.choices.get(existing).copied() {
                Some(Resolve::Keep) => Spot::Skip,
                Some(Resolve::KeepBoth) => Spot::Place(free_name(parent, name)),
                _ => Spot::Merge,
            };
        }
        match self.choices.get(existing).copied() {
            Some(Resolve::Replace) if self.discard(existing) => Spot::Place(existing.to_path_buf()),
            Some(Resolve::KeepBoth) => Spot::Place(free_name(parent, name)),
            _ => Spot::Skip,
        }
    }

    fn discard(&mut self, existing: &Path) -> bool {
        match self.bin.trash(existing) {
            Ok(held) => {
                self.replaced.push((existing.to_path_buf(), held.clone()));
                self.placed(Change::Replaced {
                    at: existing.to_path_buf(),
                    held,
                });
                true
            }
            Err(error) => {
                self.report(existing, error.reason());
                false
            }
        }
    }

    fn put_back(&mut self, from: usize) {
        let replaced: Vec<(PathBuf, Option<Held>)> = self.replaced.drain(from..).collect();
        for (at, held) in replaced.into_iter().rev() {
            let Some(held) = held else {
                self.report(&at, "it is in the trash and could not be put back".into());
                continue;
            };
            match self.bin.restore(&held, &at) {
                Ok(()) => self.send(Progress::Retracted(at)),
                Err(error) => self.report(
                    &at,
                    format!(
                        "it could not be put back and is still in the trash ({})",
                        error.reason()
                    ),
                ),
            }
        }
    }

    fn place(&mut self, src: &Path, target: &Path, moving: bool) -> bool {
        let mut from_device = None;
        if moving {
            let at = match Print::read(src) {
                Ok(at) => at,
                Err(error) => {
                    self.fail(src, &error);
                    return false;
                }
            };
            match qol_fs::rename_noreplace(src, target) {
                Ok(()) => {
                    self.placed(Change::Moved {
                        from: src.to_path_buf(),
                        to: target.to_path_buf(),
                        at,
                    });
                    return true;
                }
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                    from_device = at.device();
                }
                Err(error) => {
                    self.fail(src, &error);
                    return false;
                }
            }
        }
        if moving {
            self.cross(src, target, from_device)
        } else {
            self.copy(src, target)
        }
    }

    fn copy(&mut self, src: &Path, target: &Path) -> bool {
        let original = match TreePrint::read(src, &self.key) {
            Ok(original) => original,
            Err(error) => {
                self.fail(src, &error);
                return false;
            }
        };
        if let Err(error) = copy_to(src, target, self, false) {
            self.fail(src, &error);
            return false;
        }
        match TreePrint::read(target, &self.key) {
            Ok(tree) => self.placed(Change::Copied {
                from: src.to_path_buf(),
                to: target.to_path_buf(),
                tree,
                original,
            }),
            Err(error) => log::warn!(
                "could not read {} after copying it, so it is not in the undo history: {error}",
                target.display()
            ),
        }
        true
    }

    fn cross(&mut self, src: &Path, target: &Path, from_device: Option<u64>) -> bool {
        let copied = match copy_to(src, target, self, true) {
            Ok(copied) => copied,
            Err(error) => {
                self.fail(src, &error);
                return false;
            }
        };
        let tree = match TreePrint::read(target, &self.key) {
            Ok(tree) => tree,
            Err(error) => {
                self.report(
                    src,
                    format!(
                        "the copy could not be checked, so the original was left where it is ({})",
                        plain(&error)
                    ),
                );
                return true;
            }
        };
        self.placed(Change::Crossed {
            from: src.to_path_buf(),
            to: target.to_path_buf(),
            tree,
            source: Source::Removing,
            from_device,
        });
        let source = match remove_copied(&copied) {
            Ok(()) => Source::Removed,
            Err(error) => {
                self.fail(src, &error);
                Source::Partly
            }
        };
        self.send(Progress::Amended {
            to: target.to_path_buf(),
            source,
        });
        true
    }

    fn merge(&mut self, src: &Path, existing: &Path, moving: bool) -> bool {
        if same_object(src, existing) {
            let error = io::Error::new(io::ErrorKind::InvalidInput, ALREADY_THERE);
            self.fail(src, &error);
            return false;
        }
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
                    match self.bin.trash(&path) {
                        Ok(held) => self.placed(Change::Matched { from: path, held }),
                        Err(error) => {
                            self.report(&path, error.reason());
                            emptied = false;
                        }
                    }
                } else {
                    emptied = false;
                }
                continue;
            }
            let before = self.replaced.len();
            emptied &= match self.settle(&path, &target, existing, &name) {
                Spot::Place(landing) => {
                    let placed = self.place(&path, &landing, moving);
                    if placed {
                        self.replaced.truncate(before);
                    } else {
                        self.put_back(before);
                    }
                    placed && moving
                }
                Spot::Merge => self.merge(&path, &target, moving) && moving,
                Spot::Skip => false,
            };
        }
        if moving && emptied {
            let at = match Print::read(src) {
                Ok(at) => at,
                Err(error) => {
                    self.fail(src, &error);
                    return false;
                }
            };
            if let Err(error) = fs::remove_dir(src) {
                self.fail(src, &error);
                return false;
            }
            self.placed(Change::Emptied {
                folder: src.to_path_buf(),
                at,
            });
        }
        moving && emptied
    }

    fn identical(&self, src: &Path, target: &Path) -> bool {
        !self.choices.contains_key(target)
            && !same_entry(src, target)
            && (same_link(src, target)
                || fs::symlink_metadata(target).is_ok_and(|meta| meta.is_file())
                    && fs::symlink_metadata(src).is_ok_and(|meta| meta.is_file())
                    && same_content(src, target))
    }
}

fn same_folder(src: &Path, into: &Path) -> bool {
    src.parent().is_some_and(|parent| same_object(parent, into))
}

pub fn partial_of(target: &Path, entry: &Path) -> bool {
    let Some(first) = partial_name(target) else {
        return false;
    };
    let stem = |path: &Path| {
        let name = path.file_name()?.to_str()?;
        name.strip_suffix(PARTIAL_SUFFIX).map(str::to_owned)
    };
    entry.parent() == target.parent()
        && matches!((stem(&first), stem(entry)), (Some(first), Some(found)) if found.starts_with(&first))
}

pub(super) fn partial_variant(target: &Path, number: u32) -> Option<PathBuf> {
    let first = partial_name(target)?;
    if number <= 1 {
        return Some(first);
    }
    let name = first.file_name()?.to_str()?;
    let stem = name.strip_suffix(PARTIAL_SUFFIX)?;
    Some(first.with_file_name(format!("{stem} ({number}){PARTIAL_SUFFIX}")))
}

pub fn partial_name(target: &Path) -> Option<PathBuf> {
    let name = target.file_name()?.to_string_lossy();
    let mut cut = name.len().min(PARTIAL_NAME_BYTES);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(target.with_file_name(format!(".{}{PARTIAL_SUFFIX}", &name[..cut])))
}
