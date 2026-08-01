mod platform;

pub(crate) use platform::{set_window_fixed_size_by_title, show_normal_window_by_title};

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::runtime_config::load_gpui_runtime_config;

const COMPOSITOR_SAMPLE_INTERVAL: Duration = Duration::from_millis(16);
const COMPOSITOR_CLEAR_SAMPLES: usize = 3;
const COMPOSITOR_MAX_WAIT: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HiddenWindowsBarrier {
    pub cleared: bool,
    pub visible: usize,
    pub clear_samples: usize,
    pub elapsed: Duration,
}

pub async fn wait_for_hidden_windows(
    cx: &mut gpui::AsyncApp,
    title_prefix: &str,
) -> HiddenWindowsBarrier {
    let started = Instant::now();
    let mut clear_samples = 0;

    loop {
        cx.background_executor()
            .timer(COMPOSITOR_SAMPLE_INTERVAL)
            .await;
        let visible = cx
            .update(|_| visible_windows_by_title_prefix(title_prefix))
            .unwrap_or(usize::MAX);
        if visible == 0 {
            clear_samples += 1;
        }
        if visible != 0 {
            clear_samples = 0;
        }
        let elapsed = started.elapsed();
        if clear_samples >= COMPOSITOR_CLEAR_SAMPLES {
            return HiddenWindowsBarrier {
                cleared: true,
                visible,
                clear_samples,
                elapsed,
            };
        }
        if elapsed >= COMPOSITOR_MAX_WAIT {
            return HiddenWindowsBarrier {
                cleared: false,
                visible,
                clear_samples,
                elapsed,
            };
        }
    }
}

pub use platform::{
    capture_focus_return, configure_keepalive_window, configure_overlay_window,
    configure_pinned_window, configure_popup_window, disable_window_shadow, dump_ghost_windows,
    focus_window_by_title, hide_for_capture, hide_invisible, hide_window_by_title,
    hide_windows_by_title_prefix, make_override_redirect, park_window_by_title, pinned_window_kind,
    prepare_window_reveal_by_title, present_topmost, release_focus_by_title,
    reposition_window_by_title, restore_composite, set_window_type_dock_by_title,
    show_window_by_title, show_window_passive_by_title, sync_window_layout,
    visible_windows_by_title_prefix, window_backing_scale, window_geometry_session,
    window_holds_input_focus, window_position_by_title, WindowGeometrySession,
};

const ENV_GHOST_OPACITY: &str = "QOL_TRAY_GHOST_OPACITY";
const ENV_GHOST_COLOR: &str = "QOL_TRAY_GHOST_COLOR";

const FOCUS_REASSERT_DELAYS_MS: &[u64] = &[0, 30, 30, 30, 30, 30, 30, 30, 30, 60, 300];
const FOCUS_HELD_STREAK_STOP: u32 = 4;
const FOCUS_SETTLE_WINDOW: Duration = Duration::from_millis(180);

pub fn reassert_focus_until_held(
    title: &str,
    gen: &'static std::sync::atomic::AtomicU64,
    commit_gen: u64,
) {
    reassert_focus_until_held_with(title, gen, commit_gen, show_window_by_title);
}

pub(crate) fn reassert_normal_focus_until_held(
    title: &str,
    gen: &'static std::sync::atomic::AtomicU64,
    commit_gen: u64,
) {
    reassert_focus_until_held_with(title, gen, commit_gen, show_normal_window_by_title);
}

fn reassert_focus_until_held_with(
    title: &str,
    gen: &'static std::sync::atomic::AtomicU64,
    commit_gen: u64,
    show: fn(&str) -> bool,
) {
    if !crate::platform::should_poll_focus() {
        return;
    }
    let poll_title = title.to_string();
    let assert_title = title.to_string();
    let mut held_streak = 0u32;
    let started = Instant::now();
    crate::platform::spawn_reassert_driver(
        gen,
        commit_gen,
        FOCUS_REASSERT_DELAYS_MS,
        move || {
            let focused = window_holds_input_focus(&poll_title) == Some(true);
            let visible = focused || visible_windows_by_title_prefix(&poll_title) != 0;
            let step = focus_reassert_step(&mut held_streak, focused, visible, started.elapsed());
            if focused {
                if step == crate::platform::ReassertStep::Stop {
                    qol_runtime::probe!("FOCUS_REASSERT", "title={poll_title} step=held");
                    return step;
                }
                qol_runtime::probe!(
                    "FOCUS_REASSERT",
                    "title={poll_title} step=holding streak={held_streak}"
                );
                return step;
            }
            if step == crate::platform::ReassertStep::Stop {
                qol_runtime::probe!("FOCUS_REASSERT", "title={poll_title} step=hidden");
            }
            step
        },
        move || {
            let shown = show(&assert_title);
            #[cfg(not(debug_assertions))]
            let _ = shown;
            qol_runtime::probe!(
                "FOCUS_REASSERT",
                "title={assert_title} step=reassert shown={shown}"
            );
        },
    );
}

fn focus_reassert_step(
    held_streak: &mut u32,
    focused: bool,
    visible: bool,
    elapsed: Duration,
) -> crate::platform::ReassertStep {
    if focused {
        *held_streak = held_streak.saturating_add(1);
        if *held_streak >= FOCUS_HELD_STREAK_STOP && elapsed >= FOCUS_SETTLE_WINDOW {
            return crate::platform::ReassertStep::Stop;
        }
        return crate::platform::ReassertStep::Settled;
    }
    *held_streak = 0;
    if !visible {
        return crate::platform::ReassertStep::Stop;
    }
    crate::platform::ReassertStep::Reassert
}

pub fn set_ghost_debug(opacity: Option<f32>, color_hex: Option<&str>) {
    let runtime = load_gpui_runtime_config();
    let opacity = ghost_opacity_env().or(runtime.ghost_opacity).or(opacity);
    let runtime_color = runtime.ghost_debug_color;
    let env_color = ghost_color_env();
    let color_hex = env_color
        .as_deref()
        .or(runtime_color.as_deref())
        .or(color_hex);
    platform::set_ghost_debug(opacity, color_hex);
}

fn ghost_opacity_env() -> Option<f32> {
    std::env::var(ENV_GHOST_OPACITY)
        .ok()
        .and_then(|raw| raw.trim().parse::<f32>().ok())
}

fn ghost_color_env() -> Option<String> {
    std::env::var(ENV_GHOST_COLOR).ok()
}

thread_local! {
    static CHANGE_REASON: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(crate) fn change_reason() -> String {
    CHANGE_REASON.with(|cell| {
        let reason = cell.borrow();
        if reason.is_empty() {
            "?".to_string()
        } else {
            reason.clone()
        }
    })
}

pub struct ReasonScope(String);

pub fn reason_scope(reason: impl Into<String>) -> ReasonScope {
    let reason = reason.into();
    ReasonScope(CHANGE_REASON.with(|cell| std::mem::replace(&mut *cell.borrow_mut(), reason)))
}

impl Drop for ReasonScope {
    fn drop(&mut self) {
        CHANGE_REASON.with(|cell| *cell.borrow_mut() = std::mem::take(&mut self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_reassert_delays_cover_the_settling_window_densely() {
        let mut elapsed = 0;
        let actual: Vec<u64> = FOCUS_REASSERT_DELAYS_MS
            .iter()
            .map(|delay| {
                elapsed += delay;
                elapsed
            })
            .collect();
        assert_eq!(actual, [0, 30, 60, 90, 120, 150, 180, 210, 240, 300, 600]);
    }

    #[test]
    fn focus_reassert_does_not_settle_before_the_wm_window() {
        let mut held_streak = 0;
        for elapsed_ms in [0, 30, 60, 90, 120, 150] {
            assert_eq!(
                focus_reassert_step(
                    &mut held_streak,
                    true,
                    true,
                    Duration::from_millis(elapsed_ms),
                ),
                crate::platform::ReassertStep::Settled
            );
        }
        assert_eq!(
            focus_reassert_step(&mut held_streak, true, true, Duration::from_millis(180),),
            crate::platform::ReassertStep::Stop
        );
    }

    #[test]
    fn focus_reassert_requires_a_new_held_streak_after_a_steal() {
        let mut held_streak = 3;
        assert_eq!(
            focus_reassert_step(&mut held_streak, false, true, Duration::from_millis(120),),
            crate::platform::ReassertStep::Reassert
        );
        for elapsed_ms in [150, 180, 210] {
            assert_eq!(
                focus_reassert_step(
                    &mut held_streak,
                    true,
                    true,
                    Duration::from_millis(elapsed_ms),
                ),
                crate::platform::ReassertStep::Settled
            );
        }
        assert_eq!(
            focus_reassert_step(&mut held_streak, true, true, Duration::from_millis(240),),
            crate::platform::ReassertStep::Stop
        );
    }
}
