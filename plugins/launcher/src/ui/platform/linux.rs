use std::sync::atomic::{AtomicU64, Ordering};

static FOCUS_REASSERT_GEN: AtomicU64 = AtomicU64::new(0);

pub(super) fn show_topmost_window(target_title: &str, all_titles: &[String]) {
    qol_gpui::ghost::show_ghost_window_topmost(target_title, all_titles);
    let generation = FOCUS_REASSERT_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    qol_gpui::popup_window::reassert_focus_until_held(
        target_title,
        &FOCUS_REASSERT_GEN,
        generation,
    );
}
