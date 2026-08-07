mod settle;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

pub use settle::settled;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Recursion {
    Shallow,
    Deep,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchRoot {
    pub path: PathBuf,
    pub recursion: Recursion,
}

impl WatchRoot {
    pub fn shallow(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            recursion: Recursion::Shallow,
        }
    }

    pub fn deep(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            recursion: Recursion::Deep,
        }
    }

    fn mode(&self) -> RecursiveMode {
        match self.recursion {
            Recursion::Shallow => RecursiveMode::NonRecursive,
            Recursion::Deep => RecursiveMode::Recursive,
        }
    }
}

#[derive(Clone, Debug)]
pub enum WatchNotice {
    Changed(Vec<PathBuf>),
    Failed(String),
}

pub struct Watch {
    _watcher: RecommendedWatcher,
    rejected: Vec<RejectedRoot>,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Watch")
            .field("rejected", &self.rejected)
            .finish()
    }
}

impl Watch {
    pub fn rejected_roots(&self) -> &[RejectedRoot] {
        &self.rejected
    }
}

#[derive(Clone, Debug)]
pub struct RejectedRoot {
    pub path: PathBuf,
    pub reason: String,
}

pub fn watch(
    roots: &[WatchRoot],
    handler: impl Fn(WatchNotice) + Send + 'static,
) -> Result<Watch, WatchError> {
    if roots.is_empty() {
        return Err(WatchError::NoRoots);
    }
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) if is_mutating(&event.kind) => {
                handler(WatchNotice::Changed(event.paths));
            }
            Ok(_) => {}
            Err(error) => handler(WatchNotice::Failed(error.to_string())),
        })
        .map_err(WatchError::Watcher)?;
    let mut rejected = Vec::new();
    for root in roots {
        if let Err(error) = watcher.watch(&root.path, root.mode()) {
            rejected.push(RejectedRoot {
                path: root.path.clone(),
                reason: error.to_string(),
            });
        }
    }
    if rejected.len() == roots.len() {
        let reason = rejected
            .into_iter()
            .map(|root| format!("{}: {}", root.path.display(), root.reason))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(WatchError::NoWatchableRoot(reason));
    }
    Ok(Watch {
        _watcher: watcher,
        rejected,
    })
}

pub fn watch_file(
    path: impl AsRef<Path>,
    handler: impl Fn() + Send + 'static,
) -> Result<Watch, WatchError> {
    let path = path.as_ref();
    let file_name = path
        .file_name()
        .ok_or_else(|| WatchError::InvalidPath(path.to_path_buf()))?;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| WatchError::InvalidPath(path.to_path_buf()))?;
    let directory = std::fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf());
    let watched = directory.join(file_name);
    watch(&[WatchRoot::shallow(directory)], move |notice| {
        if let WatchNotice::Changed(paths) = notice {
            if paths.iter().any(|changed| changed == &watched) {
                handler();
            }
        }
    })
}

fn is_mutating(kind: &EventKind) -> bool {
    kind.is_create() || kind.is_modify() || kind.is_remove()
}

#[derive(Debug)]
pub enum WatchError {
    NoRoots,
    InvalidPath(PathBuf),
    NoWatchableRoot(String),
    Watcher(notify::Error),
}

impl Display for WatchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRoots => write!(formatter, "no filesystem roots were given to watch"),
            Self::InvalidPath(path) => {
                write!(formatter, "{} has no watchable parent", path.display())
            }
            Self::NoWatchableRoot(reason) => {
                write!(formatter, "no root could be watched: {reason}")
            }
            Self::Watcher(error) => write!(formatter, "filesystem watcher failed: {error}"),
        }
    }
}

impl Error for WatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Watcher(error) => Some(error),
            Self::NoRoots | Self::InvalidPath(_) | Self::NoWatchableRoot(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{watch, watch_file, WatchNotice, WatchRoot};

    const DELIVERY: Duration = Duration::from_secs(5);

    #[test]
    fn a_shallow_root_reports_files_that_appear_under_it() {
        let root = TempDir::new().unwrap();
        let (changed, events) = mpsc::channel();
        let _watch = watch(&[WatchRoot::shallow(root.path())], move |notice| {
            if let WatchNotice::Changed(paths) = notice {
                let _ = changed.send(paths);
            }
        })
        .unwrap();

        fs::write(root.path().join("model.onnx"), b"weights").unwrap();

        let paths = events.recv_timeout(DELIVERY).unwrap();
        assert!(paths.iter().any(|path| path.ends_with("model.onnx")));
    }

    #[test]
    fn a_deep_root_reports_directories_that_appear_under_it() {
        let root = TempDir::new().unwrap();
        let (changed, events) = mpsc::channel();
        let _watch = watch(&[WatchRoot::deep(root.path())], move |notice| {
            if let WatchNotice::Changed(paths) = notice {
                let _ = changed.send(paths);
            }
        })
        .unwrap();

        let nested = root.path().join("parakeet");
        fs::create_dir(&nested).unwrap();

        let paths = events.recv_timeout(DELIVERY).unwrap();
        assert!(paths.iter().any(|path| path.ends_with("parakeet")));
    }

    #[test]
    fn a_deep_root_reports_files_written_into_an_established_directory() {
        let root = TempDir::new().unwrap();
        let nested = root.path().join("parakeet");
        fs::create_dir(&nested).unwrap();
        let (changed, events) = mpsc::channel();
        let _watch = watch(&[WatchRoot::deep(root.path())], move |notice| {
            if let WatchNotice::Changed(paths) = notice {
                let _ = changed.send(paths);
            }
        })
        .unwrap();

        fs::write(nested.join("tokens.txt"), b"tokens").unwrap();

        let paths = events.recv_timeout(DELIVERY).unwrap();
        assert!(paths.iter().any(|path| path.ends_with("tokens.txt")));
    }

    #[test]
    fn watching_reports_when_nothing_can_be_watched() {
        let root = TempDir::new().unwrap();
        let cases: [(&str, Vec<WatchRoot>); 2] = [
            ("no roots", Vec::new()),
            (
                "missing root",
                vec![WatchRoot::shallow(root.path().join("absent"))],
            ),
        ];
        for (label, roots) in cases {
            assert!(watch(&roots, |_| {}).is_err(), "case: {label}");
        }
    }

    #[test]
    fn a_partially_watchable_set_keeps_the_roots_that_exist() {
        let root = TempDir::new().unwrap();
        let watch = watch(
            &[
                WatchRoot::shallow(root.path()),
                WatchRoot::shallow(root.path().join("absent")),
            ],
            |_| {},
        )
        .unwrap();

        assert_eq!(watch.rejected_roots().len(), 1);
        assert!(watch.rejected_roots()[0].path.ends_with("absent"));
    }

    #[test]
    fn a_file_watch_ignores_its_siblings() {
        let root = TempDir::new().unwrap();
        let watched = root.path().join("metadata.jsonl");
        fs::write(&watched, b"old").unwrap();
        let (changed, events) = mpsc::channel();
        let _watch = watch_file(&watched, move || {
            let _ = changed.send(());
        })
        .unwrap();

        fs::write(root.path().join("other.jsonl"), b"noise").unwrap();
        assert!(events.recv_timeout(Duration::from_millis(300)).is_err());

        fs::write(&watched, b"new").unwrap();
        events.recv_timeout(DELIVERY).unwrap();
    }
}
