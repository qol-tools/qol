use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) mod state_store;

pub const LAST_MINIMIZED_MAX_AGE_SECS: u64 = 60 * 60 * 8;

#[derive(Clone)]
pub struct MinimizedWindowRecord {
    pub window_id: String,
    pub pid: u32,
    pub process_start_ticks: u64,
    pub saved_at_unix_secs: u64,
    pub saved_rect: Option<[f64; 4]>,
}

/// Stack of minimized window records. Minimize pushes, restore pops.
pub trait MinimizedStateStore {
    /// Read the most recently minimized window without removing it.
    fn peek(&self) -> Result<Option<MinimizedWindowRecord>, String>;
    /// Push a newly minimized window onto the stack.
    fn push(&self, record: &MinimizedWindowRecord);
    /// Pop the most recently minimized window from the stack.
    fn pop(&self) -> Result<Option<MinimizedWindowRecord>, String>;
}

pub trait WindowSystem {
    fn active_window_id(&self) -> Result<Option<String>, String>;
    fn minimize_window(&self, window_id: &str) -> Result<bool, String>;
    fn window_rect(&self, window_id: &str) -> Option<[f64; 4]>;
    fn stacking_window_ids(&self) -> Result<Vec<String>, String>;
    fn is_window_id(&self, id: &str) -> bool;
    fn normalize_window_id(&self, window_id: &str) -> Option<String>;
    fn is_excluded_window_type(&self, window_id: &str) -> Result<bool, String>;
    fn is_hidden_window(&self, window_id: &str) -> Result<bool, String>;
    fn is_launcher_window(&self, window_id: &str) -> bool;
    fn activate_window(&self, window_id: &str) -> Result<bool, String>;
    fn restore_rect(&self, window_id: &str, rect: [f64; 4]) -> Result<(), String>;
    fn window_pid(&self, window_id: &str) -> Result<Option<u32>, String>;
    fn process_start_ticks(&self, pid: u32) -> Option<u64>;
}

pub fn minimize_window<S: WindowSystem, T: MinimizedStateStore>(
    system: &S,
    store: &T,
) -> Result<(), String> {
    let Some(window_id) = system.active_window_id()? else {
        trace_restore("minimize", "none", "no-active-window");
        return Ok(());
    };

    if query_or(system.is_excluded_window_type(&window_id), false) {
        trace_restore("minimize", &window_id, "excluded");
        return Ok(());
    }

    let saved_rect = system.window_rect(&window_id);

    if !system.minimize_window(&window_id)? {
        return Err("Failed to minimize window".to_string());
    }

    let recorded = push_minimized_window(system, store, &window_id, saved_rect);
    trace_restore(
        "minimize",
        &window_id,
        if recorded { "recorded" } else { "unrecorded" },
    );
    Ok(())
}

pub fn restore_window<S: WindowSystem, T: MinimizedStateStore>(
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
        trace_restore("restore", &record.window_id, "expired");
        store.pop().ok();
        return Ok(false);
    }

    if !is_record_current(system, &record)? {
        trace_restore("restore", &record.window_id, "stale");
        store.pop().ok();
        return Ok(false);
    }

    match try_restore_window(system, &record.window_id)? {
        RestoreAttempt::Restored => {
            if let Some(rect) = record.saved_rect {
                let _ = system.restore_rect(&record.window_id, rect);
            }
            store.pop().ok();
            trace_restore("restore", &record.window_id, "restored");
            return Ok(true);
        }
        RestoreAttempt::Skipped => {
            trace_restore("restore", &record.window_id, "skipped");
        }
    }

    store.pop().ok();
    Ok(false)
}

fn restore_hidden_window_from_stacking<S: WindowSystem>(system: &S) -> Result<(), String> {
    let window_ids = system.stacking_window_ids()?;

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
    window_id: &str,
) -> Result<RestoreAttempt, String> {
    if !system.is_window_id(window_id) {
        return Ok(RestoreAttempt::Skipped);
    }
    if query_or(system.is_excluded_window_type(window_id), false) {
        return Ok(RestoreAttempt::Skipped);
    }
    if system.is_launcher_window(window_id) {
        return Ok(RestoreAttempt::Skipped);
    }
    if !query_or(system.is_hidden_window(window_id), false) {
        return Ok(RestoreAttempt::Skipped);
    }
    if system.activate_window(window_id)? {
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
    window_id: &str,
    saved_rect: Option<[f64; 4]>,
) -> bool {
    let Some(window_id) = system.normalize_window_id(window_id) else {
        return false;
    };

    let Some(pid) = system.window_pid(&window_id).ok().flatten() else {
        return false;
    };

    let Some(process_start_ticks) = system.process_start_ticks(pid) else {
        return false;
    };

    let record = MinimizedWindowRecord {
        window_id,
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
