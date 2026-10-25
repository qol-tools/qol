use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::platform;

const REPORT_EVERY: Duration = Duration::from_millis(120);

#[derive(Default)]
struct Totals {
    bytes: AtomicU64,
    files: AtomicU64,
    folders: AtomicU64,
    unreadable: AtomicU64,
    done: AtomicBool,
    cancelled: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counted {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
    pub unreadable: u64,
    pub done: bool,
}

pub(crate) struct Scan {
    totals: Arc<Totals>,
}

impl Scan {
    pub(crate) fn start(roots: Vec<PathBuf>, wake: impl Fn() + Send + 'static) -> Self {
        let totals = Arc::new(Totals::default());
        let shared = Arc::clone(&totals);
        let spawned = thread::Builder::new()
            .name("qol-explorer-props".into())
            .spawn(move || {
                let mut pace = Pace {
                    wake: Box::new(wake),
                    last: Instant::now(),
                };
                for root in &roots {
                    walk(root, &shared, &mut pace);
                }
                shared.done.store(true, Ordering::Release);
                (pace.wake)();
            });
        if let Err(error) = spawned {
            log::warn!("folder size will not be counted: {error}");
            totals.unreadable.fetch_add(1, Ordering::Relaxed);
            totals.done.store(true, Ordering::Release);
        }
        Self { totals }
    }

    pub(crate) fn counted(&self) -> Counted {
        Counted {
            bytes: self.totals.bytes.load(Ordering::Relaxed),
            files: self.totals.files.load(Ordering::Relaxed),
            folders: self.totals.folders.load(Ordering::Relaxed),
            unreadable: self.totals.unreadable.load(Ordering::Relaxed),
            done: self.totals.done.load(Ordering::Acquire),
        }
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.totals.cancelled.store(true, Ordering::Relaxed);
    }
}

struct Pace {
    wake: Box<dyn Fn() + Send>,
    last: Instant,
}

impl Pace {
    fn tick(&mut self) {
        if self.last.elapsed() >= REPORT_EVERY {
            self.last = Instant::now();
            (self.wake)();
        }
    }
}

fn walk(path: &Path, totals: &Totals, pace: &mut Pace) {
    let mut pending = vec![path.to_path_buf()];
    let mut first = true;
    let mut home = None;
    while let Some(next) = pending.pop() {
        if totals.cancelled.load(Ordering::Relaxed) {
            return;
        }
        let metadata = if first {
            fs::metadata(&next).or_else(|_| fs::symlink_metadata(&next))
        } else {
            fs::symlink_metadata(&next)
        };
        let root = first;
        first = false;
        let Ok(metadata) = metadata else {
            totals.unreadable.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if !metadata.is_dir() {
            totals.files.fetch_add(1, Ordering::Relaxed);
            totals.bytes.fetch_add(metadata.len(), Ordering::Relaxed);
            pace.tick();
            continue;
        }
        totals.folders.fetch_add(1, Ordering::Relaxed);
        let volume = platform::volume(&metadata);
        if root {
            home = volume;
        } else if volume != home {
            continue;
        }
        match fs::read_dir(&next) {
            Ok(items) => pending.extend(items.filter_map(Result::ok).map(|item| item.path())),
            Err(_) => {
                totals.unreadable.fetch_add(1, Ordering::Relaxed);
            }
        }
        pace.tick();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{Counted, Scan};

    fn finished(scan: &Scan) -> Counted {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let counted = scan.counted();
            if counted.done || Instant::now() > deadline {
                return counted;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_folder_tree_adds_up_every_file_below_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join("a/b")).expect("dirs");
        fs::write(dir.path().join("one"), vec![0; 100]).expect("file");
        fs::write(dir.path().join("a/two"), vec![0; 20]).expect("file");
        fs::write(dir.path().join("a/b/three"), vec![0; 3]).expect("file");
        let scan = Scan::start(vec![dir.path().to_path_buf()], || {});
        let counted = finished(&scan);
        assert!(counted.done);
        assert_eq!(counted.bytes, 123);
        assert_eq!(counted.files, 3);
        assert_eq!(counted.folders, 3);
        assert_eq!(counted.unreadable, 0);
    }

    #[test]
    fn several_roots_share_one_running_total() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("x"), vec![0; 7]).expect("file");
        fs::write(dir.path().join("y"), vec![0; 5]).expect("file");
        let scan = Scan::start(vec![dir.path().join("x"), dir.path().join("y")], || {});
        let counted = finished(&scan);
        assert_eq!((counted.bytes, counted.files), (12, 2));
    }

    #[test]
    fn a_missing_root_counts_as_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scan = Scan::start(vec![dir.path().join("gone")], || {});
        let counted = finished(&scan);
        assert!(counted.done);
        assert_eq!((counted.bytes, counted.unreadable), (0, 1));
    }
}
