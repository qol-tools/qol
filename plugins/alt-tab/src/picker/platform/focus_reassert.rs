use std::sync::atomic::{AtomicU64, Ordering};

static FOCUS_REASSERT_GEN: AtomicU64 = AtomicU64::new(0);

pub(super) fn hold(title: &str) {
    let generation = FOCUS_REASSERT_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    qol_gpui::popup_window::reassert_focus_until_held(title, &FOCUS_REASSERT_GEN, generation);
}

pub(super) fn cancel() {
    FOCUS_REASSERT_GEN.fetch_add(1, Ordering::SeqCst);
}
