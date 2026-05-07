use anyhow::{anyhow, Result};
use notify_debouncer_full::{
    new_debouncer,
    notify::{RecommendedWatcher, RecursiveMode},
    DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const SHORTCUTS_WATCH_DEBOUNCE: Duration = Duration::from_millis(500);

type ShortcutDebouncer = Debouncer<RecommendedWatcher, RecommendedCache>;

pub struct ShortcutWatcher {
    _debouncer: Option<ShortcutDebouncer>,
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
        Self { _debouncer: None }
    }

    fn watch_path<F>(target_path: PathBuf, debounce: Duration, sync: F) -> Result<Self>
    where
        F: Fn() + Send + 'static,
    {
        let parent = target_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("shortcuts path has no parent: {}", target_path.display()))?;
        let mut debouncer = new_debouncer(debounce, None, move |result| {
            handle_debounced_events(result, &target_path, &sync);
        })?;
        debouncer.watch(parent, RecursiveMode::NonRecursive)?;
        Ok(Self {
            _debouncer: Some(debouncer),
        })
    }
}

fn handle_debounced_events<F>(result: DebounceEventResult, target_path: &Path, sync: &F)
where
    F: Fn(),
{
    let events = match result {
        Ok(events) => events,
        Err(errors) => {
            for error in errors {
                tracing::warn!("Shortcuts watcher event error: {error}");
            }
            return;
        }
    };

    if !debounced_events_include_target(&events, target_path) {
        return;
    }

    sync();
}

fn debounced_events_include_target(events: &[DebouncedEvent], target_path: &Path) -> bool {
    events.iter().any(|event| {
        event
            .event
            .paths
            .iter()
            .any(|path| path_matches_target(path, target_path))
    })
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
    use notify_debouncer_full::notify::{
        event::{ModifyKind, RenameMode},
        Event, EventKind,
    };
    use std::fs;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Instant;

    #[test]
    fn target_path_event_triggers_sync() {
        let target = PathBuf::from("/tmp/qol/shortcuts.json");
        let event = debounced_event(
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(PathBuf::from("/tmp/qol/.shortcuts.json.tmp"))
                .add_path(target.clone()),
        );

        assert!(debounced_events_include_target(&[event], &target));
    }

    #[test]
    fn partial_path_match_does_not_trigger_sync() {
        let target = PathBuf::from("/tmp/qol/shortcuts.json");
        let paths = [
            PathBuf::from("/tmp/qol/shortcuts.json.bak"),
            PathBuf::from("/tmp/qol-backup/shortcuts.json"),
            PathBuf::from("/tmp/qol/nested/shortcuts.json"),
        ];

        for path in paths {
            let event =
                debounced_event(Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path));

            assert!(!debounced_events_include_target(&[event], &target));
        }
    }

    #[test]
    fn atomic_rename_save_triggers_once() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let target = temp_dir.path().join("shortcuts.json");
        let (watcher, rx) = watcher_for_path(target.clone());

        atomic_save(&target, "alpha");
        wait_for_sync(&rx);
        assert_no_extra_sync(&rx);

        drop(watcher);
    }

    #[test]
    fn rapid_edits_within_debounce_coalesce() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let target = temp_dir.path().join("shortcuts.json");
        fs::write(&target, shortcuts_json("initial")).expect("write shortcuts");
        let (watcher, rx) = watcher_for_path(target.clone());

        for id in ["alpha", "beta", "gamma"] {
            fs::write(&target, shortcuts_json(id)).expect("write shortcuts");
            std::thread::sleep(Duration::from_millis(60));
        }

        wait_for_sync(&rx);
        assert_no_extra_sync(&rx);

        drop(watcher);
    }

    fn watcher_for_path(target: PathBuf) -> (ShortcutWatcher, Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let watcher = ShortcutWatcher::watch_path(target, SHORTCUTS_WATCH_DEBOUNCE, move || {
            let _ = tx.send(());
        })
        .expect("watch shortcuts path");
        (watcher, rx)
    }

    fn debounced_event(event: Event) -> DebouncedEvent {
        DebouncedEvent::new(event, Instant::now())
    }

    fn atomic_save(target: &Path, id: &str) {
        let temp_path = target.with_file_name(format!(".shortcuts-{id}.tmp"));
        fs::write(&temp_path, shortcuts_json(id)).expect("write temp shortcuts");
        fs::rename(temp_path, target).expect("rename shortcuts");
    }

    fn shortcuts_json(id: &str) -> String {
        format!(r#"{{"shortcuts":[{{"id":"{id}"}}]}}"#)
    }

    fn wait_for_sync(rx: &Receiver<()>) {
        rx.recv_timeout(Duration::from_secs(4))
            .expect("shortcut watcher did not sync");
    }

    fn assert_no_extra_sync(rx: &Receiver<()>) {
        assert!(rx
            .recv_timeout(SHORTCUTS_WATCH_DEBOUNCE + Duration::from_millis(250))
            .is_err());
    }
}
