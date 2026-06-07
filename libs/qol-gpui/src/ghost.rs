use std::sync::Mutex;

use gpui::*;

use crate::monitor::ActiveMonitor;
use crate::popup_window;
use crate::protocol::RuntimeEvent;

pub fn sync_window_layout(
    title: &str,
    window: &mut Window,
    origin: Point<Pixels>,
    size: Size<Pixels>,
) -> bool {
    #[cfg(target_os = "linux")]
    {
        let _ = window;
        popup_window::set_window_bounds_by_title(
            title,
            origin.x.to_f64(),
            origin.y.to_f64(),
            size.width.to_f64(),
            size.height.to_f64(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        let backing = popup_window::window_backing_scale(title);
        crate::window::resize_or_sync_scale(window, size, backing);
        popup_window::reposition_window_by_title(title, origin.x.to_f64(), origin.y.to_f64())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = origin;
        let backing = popup_window::window_backing_scale(title);
        crate::window::resize_or_sync_scale(window, size, backing);
        true
    }
}

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

pub fn ghost_window_title(prefix: &str, target: crate::window::MonitorKey) -> String {
    format!(
        "{}@{},{},{}x{}",
        prefix, target.x, target.y, target.width, target.height
    )
}

pub fn show_ghost_window(target_title: &str, all_titles: &[String]) {
    qol_runtime::probe!(
        "SHOW_GHOST",
        "target={target_title} n_titles={}",
        all_titles.len()
    );
    for title in all_titles {
        if title != target_title {
            hide_invisible(title);
        }
    }
    popup_window::show_window_by_title(target_title);
    popup_window::dump_ghost_windows(&format!(
        "show target={target_title} active_mon={:?}",
        active_monitor()
    ));
}

pub fn hide_invisible(title: &str) {
    #[cfg(target_os = "linux")]
    {
        popup_window::hide_window_invisible(title);
    }
    #[cfg(not(target_os = "linux"))]
    {
        popup_window::hide_window_by_title(title);
    }
}

pub fn reconcile(active_title: &str, all_titles: &[String]) {
    #[cfg(target_os = "linux")]
    {
        qol_runtime::probe!("RECONCILE", "active={active_title} n={}", all_titles.len());
        for title in all_titles {
            if title == active_title {
                popup_window::hide_window_by_title(title);
            } else {
                popup_window::hide_window_invisible(title);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = active_title;
        let _ = all_titles;
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

pub fn dismiss_to_ghost(prefix: &str, my_title: &str) {
    let _reason = popup_window::reason_scope("dismiss");
    let on_active = resolve_active_monitor()
        .map(|monitor| {
            ghost_window_title(
                prefix,
                crate::window::MonitorKey::from_bounds(&monitor.bounds()),
            )
        })
        .as_deref()
        == Some(my_title);
    if on_active {
        popup_window::hide_window_by_title(my_title);
    } else {
        hide_invisible(my_title);
    }
    popup_window::dump_ghost_windows(&format!(
        "dismiss title={my_title} active_mon={:?}",
        active_monitor()
    ));
}

pub fn track_dismiss<V: gpui::Focusable + 'static>(
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
    let on_dismiss_cell = std::rc::Rc::new(std::cell::RefCell::new(on_dismiss));
    let get_blur_guard = std::rc::Rc::new(get_blur_guard);
    let is_showing = std::rc::Rc::new(is_showing);
    let window_handle = window.to_async(cx).window_handle();

    let get_blur_guard_1 = get_blur_guard.clone();
    let is_showing_1 = is_showing.clone();
    let on_dismiss_1 = on_dismiss_cell.clone();
    let blur_sub = cx.on_blur(focus_handle, window, move |view, window, cx| {
        if !is_showing_1(view) {
            return;
        }
        if std::time::Instant::now() < get_blur_guard_1(view) {
            return;
        }
        (*on_dismiss_1.borrow_mut())(view, window, cx);
    });

    let get_blur_guard_2 = get_blur_guard.clone();
    let is_showing_2 = is_showing.clone();
    let on_dismiss_2 = on_dismiss_cell.clone();
    let active_sub = cx.observe_window_activation(window, move |view, window, cx| {
        if !is_showing_2(view) {
            return;
        }
        if !window.is_window_active() {
            if std::time::Instant::now() < get_blur_guard_2(view) {
                return;
            }
            (*on_dismiss_2.borrow_mut())(view, window, cx);
        }
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
