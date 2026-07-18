use qol_conventions::doctor_cli::{PROGRESS_ENV_VAR, PROGRESS_LINE_PREFIX};
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub(super) fn enable_from_env() {
    if std::env::var_os(PROGRESS_ENV_VAR).is_some() {
        ENABLED.store(true, Ordering::Relaxed);
    }
}

pub(super) fn emit(step: &str) {
    if ENABLED.load(Ordering::Relaxed) {
        eprintln!("{PROGRESS_LINE_PREFIX}{step}");
    }
}
