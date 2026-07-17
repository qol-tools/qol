use std::sync::Mutex;

use gpui::*;

use crate::monitor::ActiveMonitor;
use crate::popup_window;
use crate::protocol::RuntimeEvent;

pub use crate::popup_window::{hide_invisible, sync_window_layout};

static ACTIVE_MONITOR: Mutex<Option<ActiveMonitor>> = Mutex::new(None);

pub fn record_active_monitor(event: &RuntimeEvent) -> Option<ActiveMonitor> {
    let monitor = ActiveMonitor::from_event(event)?;
    if let Ok(mut slot) = ACTIVE_MONITOR.lock() {
        *slot = Some(monitor.clone());
    }
    Some(monitor)
}

pub fn active_monitor() -> Option<ActiveMonitor> {
    ACTIVE_MONITOR.lock().ok().and_then(|slot| slot.clone())
}

pub fn resolve_active_monitor() -> Option<ActiveMonitor> {
    active_monitor().or_else(|| {
        crate::PlatformStateClient::from_env()
            .get_state()
            .and_then(|state| state.active_monitor().map(ActiveMonitor::from_bounds))
    })
}

pub fn refresh_active_monitor_from_state() {
    let fresh = crate::PlatformStateClient::from_env()
        .get_state()
        .and_then(|state| state.active_monitor().map(ActiveMonitor::from_bounds));
    if let Ok(mut slot) = ACTIVE_MONITOR.lock() {
        *slot = fresh;
    }
}

pub fn ghost_window_title(prefix: &str, target: crate::window::MonitorKey) -> String {
    format!(
        "{}@{},{},{}x{}",
        prefix, target.x, target.y, target.width, target.height
    )
}

pub fn show_ghost_window(target_title: &str, all_titles: &[String]) {
    let reason = popup_window::change_reason();
    qol_runtime::probe!(
        "SHOW_GHOST",
        "reason={reason} target={target_title} n_titles={}",
        all_titles.len()
    );
    for title in all_titles {
        if title != target_title {
            hide_invisible(title);
        }
    }
    let shown = popup_window::show_window_by_title(target_title);
    qol_runtime::probe!(
        "SHOW_GHOST_RESULT",
        "reason={reason} target={target_title} shown={shown}"
    );
    popup_window::dump_ghost_windows(&format!(
        "show reason={reason} target={target_title} active_mon={:?}",
        active_monitor()
    ));
}

pub fn show_ghost_window_topmost(target_title: &str, all_titles: &[String]) {
    popup_window::present_topmost(target_title);
    show_ghost_window(target_title, all_titles);
}

pub fn reconcile(active_title: &str, all_titles: &[String]) {
    qol_runtime::probe!("RECONCILE", "active={active_title} n={}", all_titles.len());
    for title in all_titles {
        if title == active_title {
            popup_window::hide_window_by_title(title);
        } else {
            popup_window::hide_invisible(title);
        }
    }
    popup_window::dump_ghost_windows(&format!(
        "reconcile target={active_title} active_mon={:?}",
        active_monitor()
    ));
}

pub fn reconcile_active<F>(keys: &[crate::window::MonitorKey], title_of: F)
where
    F: Fn(crate::window::MonitorKey) -> String,
{
    let Some(active) = resolve_active_monitor() else {
        return;
    };
    let active_key = crate::window::MonitorKey::from_bounds(&active.bounds());
    let all_titles: Vec<String> = keys.iter().map(|key| title_of(*key)).collect();
    reconcile(&title_of(active_key), &all_titles);
}

pub fn reconcile_from_event<T: 'static>(
    event: &RuntimeEvent,
    active: &crate::window::ActiveWindows<T>,
    title_of: impl Fn(crate::window::MonitorKey) -> String,
    fallback: impl FnOnce() -> Option<ActiveMonitor>,
) -> bool {
    let Some(monitor) = record_active_monitor(event).or_else(fallback) else {
        return false;
    };
    let target = crate::window::target_monitor_key(Some(&monitor));
    let all_titles = active.titles_with(&title_of);
    reconcile(&title_of(target), &all_titles);
    true
}

pub fn rebuild_on_topology<T: Render + 'static>(
    event: &RuntimeEvent,
    visible: bool,
    active: &std::rc::Rc<std::cell::RefCell<crate::window::ActiveWindows<T>>>,
    cx: &mut App,
    pre_create: impl FnOnce(&mut App),
) -> bool {
    if !matches!(event, RuntimeEvent::MonitorsChanged { .. }) {
        return false;
    }
    if visible {
        qol_runtime::probe!("GHOST_TOPOLOGY", "skipped: popup visible");
        return false;
    }
    refresh_active_monitor_from_state();
    let mut stale = std::mem::take(&mut *active.borrow_mut());
    #[cfg(debug_assertions)]
    let n_stale = stale.len();
    stale.destroy_all(cx);
    pre_create(cx);
    qol_runtime::probe!("GHOST_TOPOLOGY", "rebuilt: destroyed={n_stale}");
    true
}

pub fn dismiss_to_ghost(prefix: &str, my_title: &str) {
    dismiss_to_ghost_with(my_title, |key| ghost_window_title(prefix, key));
}

pub fn dismiss_to_ghost_with(
    my_title: &str,
    title_of: impl Fn(crate::window::MonitorKey) -> String,
) {
    let _reason = popup_window::reason_scope("dismiss");
    let active_title = resolve_active_monitor()
        .map(|monitor| title_of(crate::window::MonitorKey::from_bounds(&monitor.bounds())));
    match active_title {
        Some(active_title) if active_title != my_title => {
            hide_invisible(my_title);
            popup_window::hide_window_by_title(&active_title);
        }
        _ => {
            popup_window::hide_window_by_title(my_title);
        }
    }
    popup_window::dump_ghost_windows(&format!(
        "dismiss title={my_title} active_mon={:?}",
        active_monitor()
    ));
}

fn trace_dismiss_decision(
    label: &'static str,
    event: &str,
    showing: bool,
    active: &str,
    guard: std::time::Instant,
    decision: &str,
) {
    qol_runtime::probe!(
        "GHOST_DISMISS",
        "label={label} event={event} showing={showing} active={active} guard_ms={} decision={decision}",
        guard
            .saturating_duration_since(std::time::Instant::now())
            .as_millis(),
    );
}

fn rearm_on_new_show(
    armed_for: &std::cell::Cell<Option<std::time::Instant>>,
    has_been_active: &std::cell::Cell<bool>,
    guard: std::time::Instant,
) {
    if armed_for.get() != Some(guard) {
        armed_for.set(Some(guard));
        has_been_active.set(false);
    }
}

const DISMISS_DEBOUNCE_MS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DebounceVerdict {
    Wait,
    Recover,
    Dismiss,
}

fn debounce_verdict(
    guarded: bool,
    showing: bool,
    gpui_active: bool,
    platform_focus: Option<bool>,
) -> DebounceVerdict {
    if !showing {
        return DebounceVerdict::Recover;
    }
    if guarded {
        return DebounceVerdict::Wait;
    }
    if platform_focus.unwrap_or(gpui_active) {
        DebounceVerdict::Recover
    } else {
        DebounceVerdict::Dismiss
    }
}

#[allow(clippy::too_many_arguments)]
fn schedule_debounced_dismiss<V: 'static>(
    label: &'static str,
    event: &'static str,
    window_handle: gpui::AnyWindowHandle,
    get_blur_guard: std::rc::Rc<impl Fn(&V) -> std::time::Instant + 'static>,
    is_showing: std::rc::Rc<impl Fn(&V) -> bool + 'static>,
    platform_focus: std::rc::Rc<impl Fn(&V) -> Option<bool> + 'static>,
    on_dismiss: std::rc::Rc<
        std::cell::RefCell<impl FnMut(&mut V, &mut gpui::Window, &mut gpui::Context<V>) + 'static>,
    >,
    cx: &mut gpui::Context<V>,
) {
    cx.spawn(
        move |view_handle: gpui::WeakEntity<V>, cx: &mut gpui::AsyncApp| {
            let mut cx = cx.clone();
            let get_blur_guard = get_blur_guard.clone();
            let is_showing = is_showing.clone();
            let platform_focus = platform_focus.clone();
            let on_dismiss = on_dismiss.clone();
            async move {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(DISMISS_DEBOUNCE_MS))
                        .await;
                    let view_handle = view_handle.clone();
                    let get_blur_guard = get_blur_guard.clone();
                    let is_showing = is_showing.clone();
                    let platform_focus = platform_focus.clone();
                    let on_dismiss = on_dismiss.clone();
                    let outcome = cx.update_window(window_handle, move |_, window, cx| {
                        let Some(handle) = view_handle.upgrade() else {
                            return DebounceVerdict::Recover;
                        };
                        let showing = is_showing(handle.read(cx));
                        let guarded = std::time::Instant::now() < get_blur_guard(handle.read(cx));
                        let gpui_active = window.is_window_active();
                        let focus_truth = platform_focus(handle.read(cx));
                        let verdict = debounce_verdict(guarded, showing, gpui_active, focus_truth);
                        match verdict {
                            DebounceVerdict::Wait => {
                                trace_dismiss_decision(
                                    label,
                                    event,
                                    showing,
                                    "na",
                                    std::time::Instant::now(),
                                    "debounce_guarded",
                                );
                            }
                            DebounceVerdict::Recover => {
                                let active = format!("{gpui_active} platform={focus_truth:?}");
                                trace_dismiss_decision(
                                    label,
                                    event,
                                    showing,
                                    &active,
                                    std::time::Instant::now(),
                                    "debounce_recovered",
                                );
                            }
                            DebounceVerdict::Dismiss => {
                                trace_dismiss_decision(
                                    label,
                                    event,
                                    showing,
                                    "false",
                                    std::time::Instant::now(),
                                    "dismiss",
                                );
                                handle.update(cx, |view, cx| {
                                    (*on_dismiss.borrow_mut())(view, window, cx);
                                });
                            }
                        }
                        verdict
                    });
                    if !matches!(outcome, Ok(DebounceVerdict::Wait)) {
                        break;
                    }
                }
            }
        },
    )
    .detach();
}

pub fn track_dismiss<V: gpui::Focusable + 'static>(
    label: &'static str,
    focus_handle: &gpui::FocusHandle,
    window: &mut gpui::Window,
    get_blur_guard: impl Fn(&V) -> std::time::Instant + 'static,
    is_showing: impl Fn(&V) -> bool + 'static,
    cx: &mut gpui::Context<V>,
    on_dismiss: impl FnMut(&mut V, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
) -> (
    gpui::Subscription,
    gpui::Subscription,
    Option<gpui::Task<()>>,
) {
    track_dismiss_confirmed(
        label,
        focus_handle,
        window,
        get_blur_guard,
        is_showing,
        |_| None,
        cx,
        on_dismiss,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn track_dismiss_confirmed<V: gpui::Focusable + 'static>(
    label: &'static str,
    focus_handle: &gpui::FocusHandle,
    window: &mut gpui::Window,
    get_blur_guard: impl Fn(&V) -> std::time::Instant + 'static,
    is_showing: impl Fn(&V) -> bool + 'static,
    platform_focus: impl Fn(&V) -> Option<bool> + 'static,
    cx: &mut gpui::Context<V>,
    on_dismiss: impl FnMut(&mut V, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
) -> (
    gpui::Subscription,
    gpui::Subscription,
    Option<gpui::Task<()>>,
) {
    let on_dismiss_cell = std::rc::Rc::new(std::cell::RefCell::new(on_dismiss));
    let get_blur_guard = std::rc::Rc::new(get_blur_guard);
    let is_showing = std::rc::Rc::new(is_showing);
    let platform_focus = std::rc::Rc::new(platform_focus);
    let has_been_active = std::rc::Rc::new(std::cell::Cell::new(false));
    let armed_for = std::rc::Rc::new(std::cell::Cell::new(None::<std::time::Instant>));
    let window_handle = window.to_async(cx).window_handle();

    let get_blur_guard_1 = get_blur_guard.clone();
    let is_showing_1 = is_showing.clone();
    let platform_focus_1 = platform_focus.clone();
    let on_dismiss_1 = on_dismiss_cell.clone();
    let has_been_active_1 = has_been_active.clone();
    let armed_for_1 = armed_for.clone();
    let blur_sub = cx.on_blur(focus_handle, window, move |view, window, cx| {
        let showing = is_showing_1(view);
        let guard = get_blur_guard_1(view);
        rearm_on_new_show(&armed_for_1, &has_been_active_1, guard);
        if !showing {
            trace_dismiss_decision(label, "blur", showing, "na", guard, "skip_hidden");
            return;
        }
        if !has_been_active_1.get() {
            trace_dismiss_decision(label, "blur", showing, "na", guard, "skip_settling");
            return;
        }
        if window.is_window_active() {
            trace_dismiss_decision(label, "blur", showing, "true", guard, "skip_active");
            return;
        }
        trace_dismiss_decision(label, "blur", showing, "false", guard, "debounce_scheduled");
        schedule_debounced_dismiss(
            label,
            "blur",
            window_handle,
            get_blur_guard_1.clone(),
            is_showing_1.clone(),
            platform_focus_1.clone(),
            on_dismiss_1.clone(),
            cx,
        );
    });

    let get_blur_guard_2 = get_blur_guard.clone();
    let is_showing_2 = is_showing.clone();
    let platform_focus_2 = platform_focus.clone();
    let on_dismiss_2 = on_dismiss_cell.clone();
    let has_been_active_2 = has_been_active.clone();
    let armed_for_2 = armed_for.clone();
    let active_sub = cx.observe_window_activation(window, move |view, window, cx| {
        let showing = is_showing_2(view);
        let active = window.is_window_active();
        let guard = get_blur_guard_2(view);
        rearm_on_new_show(&armed_for_2, &has_been_active_2, guard);
        if !showing {
            let active = if active { "true" } else { "false" };
            trace_dismiss_decision(label, "activation", showing, active, guard, "skip_hidden");
            return;
        }
        if active {
            has_been_active_2.set(true);
            trace_dismiss_decision(label, "activation", showing, "true", guard, "skip_active");
            return;
        }
        if !has_been_active_2.get() {
            trace_dismiss_decision(
                label,
                "activation",
                showing,
                "false",
                guard,
                "skip_settling",
            );
            return;
        }
        trace_dismiss_decision(
            label,
            "activation",
            showing,
            "false",
            guard,
            "debounce_scheduled",
        );
        schedule_debounced_dismiss(
            label,
            "activation",
            window_handle,
            get_blur_guard_2.clone(),
            is_showing_2.clone(),
            platform_focus_2.clone(),
            on_dismiss_2.clone(),
            cx,
        );
    });

    let mut poll_task = None;
    if crate::platform::should_poll_focus() {
        let on_dismiss_3 = on_dismiss_cell.clone();
        let get_blur_guard_3 = get_blur_guard.clone();
        let is_showing_3 = is_showing.clone();
        poll_task = Some(
            cx.spawn(move |view_handle: WeakEntity<V>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                let on_dismiss_3 = on_dismiss_3.clone();
                let get_blur_guard_3 = get_blur_guard_3.clone();
                let is_showing_3 = is_showing_3.clone();
                async move {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(100))
                            .await;
                        let upgrade_result =
                            cx.update(|cx| view_handle.upgrade().map(|h| is_showing_3(h.read(cx))));
                        let Some(showing) = upgrade_result.ok().flatten() else {
                            break;
                        };
                        if !showing {
                            continue;
                        }
                        let guarded = cx
                            .update(|cx| {
                                if let Some(view_handle) = view_handle.upgrade() {
                                    let view = view_handle.read(cx);
                                    std::time::Instant::now() < get_blur_guard_3(view)
                                } else {
                                    false
                                }
                            })
                            .unwrap_or(false);
                        if guarded {
                            continue;
                        }
                        let has_focus = cx
                            .background_spawn(async { crate::platform::has_process_focus() })
                            .await;
                        if has_focus {
                            continue;
                        }
                        trace_dismiss_decision(
                            label,
                            "poll",
                            true,
                            "false",
                            std::time::Instant::now(),
                            "dismiss",
                        );
                        let view_handle_clone = view_handle.clone();
                        let on_dismiss_clone = on_dismiss_3.clone();
                        let _ = cx.update_window(window_handle, move |_, window, cx| {
                            if let Some(view_handle) = view_handle_clone.upgrade() {
                                view_handle.update(cx, |view, cx| {
                                    (*on_dismiss_clone.borrow_mut())(view, window, cx);
                                });
                            }
                        });
                    }
                }
            }),
        );
    }

    (blur_sub, active_sub, poll_task)
}

#[cfg(test)]
mod tests {
    use super::{debounce_verdict, DebounceVerdict};

    #[test]
    fn debounce_recheck_dismisses_only_when_truly_inactive() {
        let cases = [
            (
                "showing, gpui inactive, no platform truth: dismiss",
                false,
                true,
                false,
                None,
                DebounceVerdict::Dismiss,
            ),
            (
                "showing, gpui active, no platform truth: recovered",
                false,
                true,
                true,
                None,
                DebounceVerdict::Recover,
            ),
            (
                "showing, gpui inactive, platform holds focus: recovered",
                false,
                true,
                false,
                Some(true),
                DebounceVerdict::Recover,
            ),
            (
                "showing, gpui active, platform lost focus: dismiss",
                false,
                true,
                true,
                Some(false),
                DebounceVerdict::Dismiss,
            ),
            (
                "no longer showing, still inactive: nothing to dismiss",
                false,
                false,
                false,
                Some(false),
                DebounceVerdict::Recover,
            ),
            (
                "no longer showing, focus held: nothing to dismiss",
                false,
                false,
                true,
                Some(true),
                DebounceVerdict::Recover,
            ),
            (
                "guarded, showing, focus not yet granted: keep waiting",
                true,
                true,
                false,
                Some(false),
                DebounceVerdict::Wait,
            ),
            (
                "guarded, showing, no platform truth, gpui inactive: keep waiting",
                true,
                true,
                false,
                None,
                DebounceVerdict::Wait,
            ),
            (
                "guarded but no longer showing: nothing to dismiss",
                true,
                false,
                false,
                Some(false),
                DebounceVerdict::Recover,
            ),
        ];
        for (case, guarded, showing, gpui_active, platform_focus, expected) in cases {
            assert_eq!(
                debounce_verdict(guarded, showing, gpui_active, platform_focus),
                expected,
                "case: {case}"
            );
        }
    }
}
