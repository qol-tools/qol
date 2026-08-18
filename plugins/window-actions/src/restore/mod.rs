use qol_windowing::{WindowId, WindowOps, WindowRect};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) mod state_store;

pub(crate) const LAST_MINIMIZED_MAX_AGE_SECS: u64 = 60 * 60 * 8;

#[derive(Clone)]
pub(crate) struct MinimizedWindowRecord {
    pub window_id: WindowId,
    pub pid: u32,
    pub process_start_ticks: u64,
    pub saved_at_unix_secs: u64,
    pub saved_rect: Option<WindowRect>,
}

/// Stack of minimized window records. Minimize pushes, restore pops.
pub(crate) trait MinimizedStateStore {
    /// Read the most recently minimized window without removing it.
    fn peek(&self) -> Result<Option<MinimizedWindowRecord>, String>;
    /// Push a newly minimized window onto the stack.
    fn push(&self, record: &MinimizedWindowRecord);
    /// Pop the most recently minimized window from the stack.
    fn pop(&self) -> Result<Option<MinimizedWindowRecord>, String>;
}

/// Plugin-specific window queries layered over the shared [`WindowOps`]
/// contract. Window identity and the common operations (geometry, focus,
/// minimize) come from the shared trait; restore-only policy lives here.
pub(crate) trait WindowSystem: WindowOps {
    fn is_excluded_window_type(&self, window_id: &WindowId) -> Result<bool, String>;
    fn is_hidden_window(&self, window_id: &WindowId) -> Result<bool, String>;
    fn is_launcher_window(&self, window_id: &WindowId) -> bool;
    fn window_pid(&self, window_id: &WindowId) -> Result<Option<u32>, String>;
    fn process_start_ticks(&self, pid: u32) -> Option<u64>;
}

pub(crate) fn minimize_window<S: WindowSystem, T: MinimizedStateStore>(
    system: &S,
    store: &T,
) -> Result<(), String> {
    let Some(window_id) = system.active_window_id()? else {
        trace_restore("minimize", "none", "no-active-window");
        return Ok(());
    };

    if query_or(system.is_excluded_window_type(&window_id), false) {
        trace_restore("minimize", window_id.as_str(), "excluded");
        return Ok(());
    }

    let saved_rect = system.window_geometry(&window_id).ok().flatten();

    if !system.minimize_window(&window_id)? {
        return Err("Failed to minimize window".to_string());
    }

    let recorded = push_minimized_window(system, store, &window_id, saved_rect);
    trace_restore(
        "minimize",
        window_id.as_str(),
        if recorded { "recorded" } else { "unrecorded" },
    );
    Ok(())
}

pub(crate) fn restore_window<S: WindowSystem, T: MinimizedStateStore>(
    system: &S,
    store: &T,
) -> Result<(), String> {
    if restore_last_minimized_window(system, store)? {
        return Ok(());
    }

    restore_hidden_window_from_stacking(system)?;
    Ok(())
}

enum RestoreAttempt {
    Restored,
    Skipped,
}

fn restore_last_minimized_window<S: WindowSystem, T: MinimizedStateStore>(
    system: &S,
    store: &T,
) -> Result<bool, String> {
    let Some(record) = store.peek()? else {
        trace_restore("restore", "none", "empty");
        return Ok(false);
    };

    if is_record_expired(&record) {
        trace_restore("restore", record.window_id.as_str(), "expired");
        store.pop().ok();
        return Ok(false);
    }

    if !is_record_current(system, &record)? {
        trace_restore("restore", record.window_id.as_str(), "stale");
        store.pop().ok();
        return Ok(false);
    }

    match try_restore_window(system, &record.window_id)? {
        RestoreAttempt::Restored => {
            if let Some(rect) = record.saved_rect {
                let _ = system.move_resize(&record.window_id, rect);
            }
            store.pop().ok();
            trace_restore("restore", record.window_id.as_str(), "restored");
            return Ok(true);
        }
        RestoreAttempt::Skipped => {
            trace_restore("restore", record.window_id.as_str(), "skipped");
        }
    }

    store.pop().ok();
    Ok(false)
}

fn restore_hidden_window_from_stacking<S: WindowSystem>(system: &S) -> Result<(), String> {
    let window_ids = system.enumerate_windows()?;

    for window_id in window_ids {
        match try_restore_window(system, &window_id)? {
            RestoreAttempt::Restored => break,
            RestoreAttempt::Skipped => {}
        }
    }

    Ok(())
}

fn try_restore_window<S: WindowSystem>(
    system: &S,
    window_id: &WindowId,
) -> Result<RestoreAttempt, String> {
    if query_or(system.is_excluded_window_type(window_id), false) {
        return Ok(RestoreAttempt::Skipped);
    }
    if system.is_launcher_window(window_id) {
        return Ok(RestoreAttempt::Skipped);
    }
    if !query_or(system.is_hidden_window(window_id), false) {
        return Ok(RestoreAttempt::Skipped);
    }
    if system.focus_window(window_id)? {
        Ok(RestoreAttempt::Restored)
    } else {
        Ok(RestoreAttempt::Skipped)
    }
}

fn query_or(value: Result<bool, String>, fallback: bool) -> bool {
    value.unwrap_or(fallback)
}

fn push_minimized_window<S: WindowSystem, T: MinimizedStateStore>(
    system: &S,
    store: &T,
    window_id: &WindowId,
    saved_rect: Option<WindowRect>,
) -> bool {
    let Some(pid) = system.window_pid(window_id).ok().flatten() else {
        return false;
    };

    let Some(process_start_ticks) = system.process_start_ticks(pid) else {
        return false;
    };

    let record = MinimizedWindowRecord {
        window_id: window_id.clone(),
        pid,
        process_start_ticks,
        saved_at_unix_secs: current_unix_secs(),
        saved_rect,
    };

    store.push(&record);
    true
}

fn trace_restore(phase: &str, target: &str, outcome: &str) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "WINACT_RESTORE",
        "phase={phase} target={target} outcome={outcome}"
    );
    #[cfg(not(debug_assertions))]
    let _ = (phase, target, outcome);
}

fn is_record_expired(record: &MinimizedWindowRecord) -> bool {
    current_unix_secs().saturating_sub(record.saved_at_unix_secs) > LAST_MINIMIZED_MAX_AGE_SECS
}

fn is_record_current<S: WindowSystem>(
    system: &S,
    record: &MinimizedWindowRecord,
) -> Result<bool, String> {
    let Some(pid) = system.window_pid(&record.window_id)? else {
        return Ok(false);
    };
    if pid != record.pid {
        return Ok(false);
    }

    let Some(start_ticks) = system.process_start_ticks(pid) else {
        return Ok(false);
    };
    Ok(start_ticks == record.process_start_ticks)
}

fn current_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const WINDOW_ID: &str = "0x1";

    fn window_id() -> WindowId {
        WindowId::parse(WINDOW_ID).unwrap()
    }

    /// The tri-state a `window_geometry` backend may report under the
    /// WindowOps contract.
    enum Geometry {
        /// Backend queried and found the window.
        Rect(WindowRect),
        /// Backend queried and the window is gone.
        Gone,
        /// Backend cannot report geometry (unsupported).
        Unsupported,
    }

    struct FakeWindowSystem {
        geometry: Geometry,
        move_resize_ok: bool,
        move_resize_calls: RefCell<Vec<(WindowId, WindowRect)>>,
    }

    impl FakeWindowSystem {
        fn new(geometry: Geometry) -> Self {
            Self {
                geometry,
                move_resize_ok: true,
                move_resize_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl WindowOps for FakeWindowSystem {
        fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
            Ok(vec![])
        }

        fn window_geometry(&self, _window_id: &WindowId) -> Result<Option<WindowRect>, String> {
            match &self.geometry {
                Geometry::Rect(rect) => Ok(Some(*rect)),
                Geometry::Gone => Ok(None),
                Geometry::Unsupported => Err("not implemented".to_string()),
            }
        }

        fn move_resize(&self, window_id: &WindowId, rect: WindowRect) -> Result<(), String> {
            self.move_resize_calls
                .borrow_mut()
                .push((window_id.clone(), rect));
            if self.move_resize_ok {
                Ok(())
            } else {
                Err("not implemented".to_string())
            }
        }

        fn focus_window(&self, _window_id: &WindowId) -> Result<bool, String> {
            Ok(true)
        }

        fn minimize_window(&self, _window_id: &WindowId) -> Result<bool, String> {
            Ok(true)
        }

        fn restore_window(&self, window_id: &WindowId) -> Result<bool, String> {
            self.focus_window(window_id)
        }

        fn active_window_id(&self) -> Result<Option<WindowId>, String> {
            Ok(Some(window_id()))
        }
    }

    impl WindowSystem for FakeWindowSystem {
        fn is_excluded_window_type(&self, _window_id: &WindowId) -> Result<bool, String> {
            Ok(false)
        }

        fn is_hidden_window(&self, _window_id: &WindowId) -> Result<bool, String> {
            Ok(true)
        }

        fn is_launcher_window(&self, _window_id: &WindowId) -> bool {
            false
        }

        fn window_pid(&self, _window_id: &WindowId) -> Result<Option<u32>, String> {
            Ok(Some(1))
        }

        fn process_start_ticks(&self, _pid: u32) -> Option<u64> {
            Some(1)
        }
    }

    #[derive(Default)]
    struct FakeStore {
        records: RefCell<Vec<MinimizedWindowRecord>>,
    }

    impl MinimizedStateStore for FakeStore {
        fn peek(&self) -> Result<Option<MinimizedWindowRecord>, String> {
            Ok(self.records.borrow().last().cloned())
        }

        fn push(&self, record: &MinimizedWindowRecord) {
            self.records.borrow_mut().push(record.clone());
        }

        fn pop(&self) -> Result<Option<MinimizedWindowRecord>, String> {
            Ok(self.records.borrow_mut().pop())
        }
    }

    #[test]
    fn minimize_saves_rect_only_for_ok_some_geometry() {
        let saved = WindowRect::from_array([10.0, 20.0, 800.0, 600.0]);
        let cases = [
            ("window found", Geometry::Rect(saved), Some(saved)),
            ("window gone", Geometry::Gone, None),
            ("unsupported", Geometry::Unsupported, None),
        ];
        for (name, geometry, expected_rect) in cases {
            let system = FakeWindowSystem::new(geometry);
            let store = FakeStore::default();
            assert!(
                minimize_window(&system, &store).is_ok(),
                "{name}: minimize must not fail on a non-readable geometry"
            );
            let record = store
                .peek()
                .unwrap()
                .unwrap_or_else(|| panic!("{name}: minimize must push a record"));
            assert_eq!(record.saved_rect, expected_rect, "{name}");
        }
    }

    #[test]
    fn restore_round_trips_saved_rect_and_tolerates_move_resize_errors() {
        let saved = WindowRect::from_array([10.0, 20.0, 800.0, 600.0]);
        for move_resize_ok in [true, false] {
            let mut system = FakeWindowSystem::new(Geometry::Rect(saved));
            system.move_resize_ok = move_resize_ok;
            let store = FakeStore::default();
            minimize_window(&system, &store).unwrap();
            assert!(restore_window(&system, &store).is_ok());
            let calls = system.move_resize_calls.borrow();
            assert_eq!(calls.len(), 1, "move_resize_ok={move_resize_ok}");
            assert_eq!(calls[0].0.as_str(), WINDOW_ID);
            assert_eq!(calls[0].1, saved);
            assert!(
                store.peek().unwrap().is_none(),
                "restore must pop the record, move_resize_ok={move_resize_ok}"
            );
        }
    }

    #[test]
    fn restore_without_saved_rect_skips_move_resize() {
        let system = FakeWindowSystem::new(Geometry::Gone);
        let store = FakeStore::default();
        minimize_window(&system, &store).unwrap();
        assert!(restore_window(&system, &store).is_ok());
        assert!(
            system.move_resize_calls.borrow().is_empty(),
            "no saved rect means no move_resize"
        );
    }
}
