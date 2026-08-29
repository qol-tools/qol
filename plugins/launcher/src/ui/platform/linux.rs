use std::sync::atomic::{AtomicU64, Ordering};

use qol_gpui::platform::ReassertStep;

static FOCUS_REASSERT_GEN: AtomicU64 = AtomicU64::new(0);
static HOLD_GEN: AtomicU64 = AtomicU64::new(0);

const HOLD_RETRY_DELAYS_MS: &[u64] = &[30, 30, 30, 30, 60, 120, 240];

pub(super) fn show_topmost_window(target_title: &str, all_titles: &[String]) {
    qol_gpui::ghost::show_ghost_window_topmost(target_title, all_titles);
    let generation = FOCUS_REASSERT_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    qol_gpui::popup_window::reassert_focus_until_held(
        target_title,
        &FOCUS_REASSERT_GEN,
        generation,
    );
    start_hold_ladder(target_title);
}

fn start_hold_ladder(title: &str) {
    if qol_gpui::popup_window::hold_input(title) {
        return;
    }
    let commit_gen = HOLD_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let retry_title = title.to_string();
    qol_gpui::platform::spawn_reassert_driver(
        &HOLD_GEN,
        commit_gen,
        HOLD_RETRY_DELAYS_MS,
        move || {
            if qol_gpui::popup_window::input_held() {
                qol_runtime::probe!("INPUT_HOLD_LADDER", "step=held");
                return ReassertStep::Stop;
            }
            ReassertStep::Reassert
        },
        move || {
            qol_gpui::popup_window::hold_input(&retry_title);
        },
    );
}
