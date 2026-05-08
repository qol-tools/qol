use anyhow::{anyhow, Result};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(crate) const SHORTCUTS_WATCH_DEBOUNCE: Duration = Duration::from_millis(350);

type EventResult = notify::Result<Event>;

pub struct ShortcutWatcher {
    inner: Option<Inner>,
}

struct Inner {
    _watcher: RecommendedWatcher,
    handle: Option<JoinHandle<()>>,
    shutdown_tx: Sender<EventResult>,
}

impl ShortcutWatcher {
    pub fn start() -> Self {
        let target_path = match crate::paths::shortcuts_path() {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    "Skipping shortcuts watcher: failed to resolve shortcuts path: {error}"
                );
                return Self::inactive();
            }
        };

        match Self::watch_path(
            target_path,
            SHORTCUTS_WATCH_DEBOUNCE,
            crate::features::launcher_apps::trigger_full_sync,
        ) {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::warn!("Skipping shortcuts watcher: {error}");
                Self::inactive()
            }
        }
    }

    fn inactive() -> Self {
        Self { inner: None }
    }

    fn watch_path<F>(target_path: PathBuf, debounce: Duration, on_change: F) -> Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let parent = target_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("shortcuts path has no parent: {}", target_path.display()))?;

        if let Some(dir) = parent.as_path().to_str() {
            std::fs::create_dir_all(dir).ok();
        }

        let (tx, rx) = mpsc::channel::<EventResult>();
        let watcher_tx = tx.clone();
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
            let _ = watcher_tx.send(res);
        })?;
        watcher.watch(&parent, RecursiveMode::NonRecursive)?;

        let handle = thread::Builder::new()
            .name("qol-shortcuts-watcher".into())
            .spawn(move || run_event_loop(rx, target_path, debounce, on_change))?;

        Ok(Self {
            inner: Some(Inner {
                _watcher: watcher,
                handle: Some(handle),
                shutdown_tx: tx,
            }),
        })
    }
}

impl Drop for ShortcutWatcher {
    fn drop(&mut self) {
        let Some(mut inner) = self.inner.take() else {
            return;
        };
        let Some(handle) = inner.handle.take() else {
            return;
        };
        let _ = inner
            .shutdown_tx
            .send(Err(notify::Error::generic("shutdown")));
        drop(inner._watcher);
        let _ = handle.join();
    }
}

fn run_event_loop<F>(
    rx: Receiver<EventResult>,
    target_path: PathBuf,
    debounce: Duration,
    on_change: F,
) where
    F: Fn(),
{
    loop {
        let first = match rx.recv() {
            Ok(value) => value,
            Err(_) => return,
        };

        if is_shutdown(&first) {
            return;
        }

        let mut matched = event_matches_target(&first, &target_path);

        loop {
            match rx.recv_timeout(debounce) {
                Ok(next) => {
                    if is_shutdown(&next) {
                        return;
                    }
                    if event_matches_target(&next, &target_path) {
                        matched = true;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        if matched {
            on_change();
        }
    }
}

fn is_shutdown(event: &EventResult) -> bool {
    matches!(event, Err(error) if matches!(error.kind, notify::ErrorKind::Generic(ref msg) if msg == "shutdown"))
}

fn event_matches_target(event: &EventResult, target_path: &Path) -> bool {
    let Ok(event) = event else {
        return false;
    };
    event
        .paths
        .iter()
        .any(|path| path_matches_target(path, target_path))
}

fn path_matches_target(path: &Path, target_path: &Path) -> bool {
    if path == target_path {
        return true;
    }
    if path.file_name() != target_path.file_name() {
        return false;
    }
    parents_match(path.parent(), target_path.parent())
}

fn parents_match(path_parent: Option<&Path>, target_parent: Option<&Path>) -> bool {
    let Some(path_parent) = path_parent else {
        return false;
    };
    let Some(target_parent) = target_parent else {
        return false;
    };
    if path_parent == target_parent {
        return true;
    }
    let Ok(path_parent) = path_parent.canonicalize() else {
        return false;
    };
    let Ok(target_parent) = target_parent.canonicalize() else {
        return false;
    };
    path_parent == target_parent
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const TEST_DEBOUNCE: Duration = Duration::from_millis(150);
    const SETTLE: Duration = Duration::from_millis(450);

    struct Counter {
        watcher: Option<ShortcutWatcher>,
        count: Arc<AtomicUsize>,
    }

    impl Counter {
        fn new(target: PathBuf, debounce: Duration) -> Self {
            let count = Arc::new(AtomicUsize::new(0));
            let count_clone = count.clone();
            let watcher = ShortcutWatcher::watch_path(target, debounce, move || {
                count_clone.fetch_add(1, Ordering::SeqCst);
            })
            .expect("watch_path");
            Self {
                watcher: Some(watcher),
                count,
            }
        }

        fn fires(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }

        fn shutdown(&mut self) {
            self.watcher.take();
        }

        fn wait_for(&self, expected: usize) {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while self.fires() < expected && std::time::Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn shortcuts_json(id: &str) -> String {
        format!(r#"{{"shortcuts":[{{"id":"{id}"}}]}}"#)
    }

    fn atomic_save(target: &Path, id: &str) {
        let temp_path = target.with_file_name(format!(".shortcuts-{id}.tmp"));
        fs::write(&temp_path, shortcuts_json(id)).expect("write temp shortcuts");
        fs::rename(temp_path, target).expect("rename shortcuts");
    }

    #[test]
    fn target_path_event_matches() {
        let target = PathBuf::from("/tmp/qol/shortcuts.json");
        let event: EventResult = Ok(Event::default()
            .add_path(PathBuf::from("/tmp/qol/.shortcuts.json.tmp"))
            .add_path(target.clone()));
        assert!(event_matches_target(&event, &target));
    }

    #[test]
    fn unrelated_paths_do_not_match() {
        let target = PathBuf::from("/tmp/qol/shortcuts.json");
        let cases = [
            "/tmp/qol/shortcuts.json.bak",
            "/tmp/qol-backup/shortcuts.json",
            "/tmp/qol/nested/shortcuts.json",
            "/tmp/qol/other.json",
        ];
        for raw in cases {
            let event: EventResult = Ok(Event::default().add_path(PathBuf::from(raw)));
            assert!(
                !event_matches_target(&event, &target),
                "should not match: {raw}"
            );
        }
    }

    #[test]
    fn shutdown_sentinel_is_recognized() {
        let event: EventResult = Err(notify::Error::generic("shutdown"));
        assert!(is_shutdown(&event));

        let other: EventResult = Err(notify::Error::generic("something else"));
        assert!(!is_shutdown(&other));
    }

    #[test]
    fn rapid_writes_collapse_into_single_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("shortcuts.json");
        fs::write(&target, shortcuts_json("seed")).expect("seed");

        let mut counter = Counter::new(target.clone(), TEST_DEBOUNCE);
        thread::sleep(Duration::from_millis(50));

        for id in ["alpha", "beta", "gamma", "delta"] {
            fs::write(&target, shortcuts_json(id)).expect("write");
            thread::sleep(Duration::from_millis(30));
        }

        counter.wait_for(1);
        thread::sleep(SETTLE);

        let observed = counter.fires();
        assert!(
            (1..=2).contains(&observed),
            "rapid writes should collapse to ~1 reload, got {observed}"
        );
        counter.shutdown();
    }

    #[test]
    fn atomic_rename_save_triggers_one_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("shortcuts.json");

        let mut counter = Counter::new(target.clone(), TEST_DEBOUNCE);
        thread::sleep(Duration::from_millis(50));

        atomic_save(&target, "alpha");
        counter.wait_for(1);
        thread::sleep(SETTLE);

        let observed = counter.fires();
        assert!(
            (1..=2).contains(&observed),
            "atomic rename should produce ~1 reload, got {observed}"
        );
        counter.shutdown();
    }

    #[test]
    fn separate_saves_each_trigger_a_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("shortcuts.json");

        let mut counter = Counter::new(target.clone(), TEST_DEBOUNCE);
        thread::sleep(Duration::from_millis(50));

        atomic_save(&target, "first");
        counter.wait_for(1);
        thread::sleep(SETTLE);

        atomic_save(&target, "second");
        counter.wait_for(2);
        thread::sleep(SETTLE);

        let observed = counter.fires();
        assert!(
            observed >= 2,
            "two distinct saves should each trigger reload, got {observed}"
        );
        counter.shutdown();
    }

    #[test]
    fn unrelated_sibling_writes_do_not_trigger_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("shortcuts.json");

        let mut counter = Counter::new(target.clone(), TEST_DEBOUNCE);
        thread::sleep(Duration::from_millis(50));

        fs::write(dir.path().join("other.json"), "{}").expect("write sibling");
        fs::write(dir.path().join("notes.txt"), "hello").expect("write sibling");
        thread::sleep(SETTLE);

        assert_eq!(
            counter.fires(),
            0,
            "writes to non-target siblings must not trigger a reload"
        );
        counter.shutdown();
    }

    #[test]
    fn drop_joins_background_thread() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("shortcuts.json");

        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        let watcher = ShortcutWatcher::watch_path(target.clone(), TEST_DEBOUNCE, move || {
            count_clone.fetch_add(1, Ordering::SeqCst);
        })
        .expect("watch_path");

        thread::sleep(Duration::from_millis(50));
        drop(watcher);

        fs::write(&target, "{}").expect("post-drop write");
        thread::sleep(SETTLE);

        assert_eq!(
            count.load(Ordering::SeqCst),
            0,
            "writes after drop must not invoke callback"
        );
    }

    #[test]
    fn missing_parent_directory_errors() {
        let target = PathBuf::from("/nonexistent/qol-tray-test-watcher/shortcuts.json");
        let result = ShortcutWatcher::watch_path(target, TEST_DEBOUNCE, || {});
        assert!(result.is_err(), "watching a non-existent parent must error");
    }
}
