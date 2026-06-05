use std::sync::Mutex;
use std::time::Duration;

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

pub fn ghost_window_title(prefix: &str, target: crate::window::MonitorKey) -> String {
    format!(
        "{}@{},{},{}x{}",
        prefix, target.x, target.y, target.width, target.height
    )
}

pub fn show_ghost_window(target_title: &str, all_titles: &[String]) {
    crate::probe::probe(
        "SHOW_GHOST",
        &format!("target={target_title} n_titles={}", all_titles.len()),
    );
    for title in all_titles {
        if title != target_title {
            hide_non_active(title);
        }
    }
    popup_window::show_window_by_title(target_title);
    popup_window::dump_ghost_windows(&format!(
        "show target={target_title} active_mon={:?}",
        active_monitor()
    ));
}

fn hide_non_active(title: &str) {
    #[cfg(target_os = "linux")]
    {
        popup_window::hide_window_invisible(title);
    }
    #[cfg(not(target_os = "linux"))]
    {
        popup_window::hide_window_by_title(title);
    }
}

pub fn active_monitor_changed(target_title: &str, all_titles: &[String]) {
    #[cfg(target_os = "linux")]
    {
        crate::probe::probe(
            "AMC",
            &format!(
                "active_visible={target_title} others_invisible n={}",
                all_titles.len()
            ),
        );
        for title in all_titles {
            if title != target_title {
                hide_non_active(title);
            }
        }
        popup_window::hide_window_by_title(target_title);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = all_titles;
    }
    popup_window::dump_ghost_windows(&format!(
        "amc target={target_title} active_mon={:?}",
        active_monitor()
    ));
}

pub fn spawn_boot_reassert(cx: &mut App, title: String, bounds: Bounds<Pixels>) {
    #[cfg(target_os = "linux")]
    {
        cx.spawn(move |cx: &mut AsyncApp| {
            let cx = cx.clone();
            async move {
                let configured = reassert_until_configured(&cx, &title, bounds).await;
                if configured {
                    rehide_after_settle(&cx, &title).await;
                }
                #[cfg(debug_assertions)]
                if !configured {
                    eprintln!("[ghost/boot] failed to configure and hide window {title:?}");
                }
            }
        })
        .detach();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = bounds;
        popup_window::configure_popup_window(&title);
        popup_window::hide_window_by_title(&title);
    }
}

#[cfg(target_os = "linux")]
async fn reassert_until_configured(cx: &AsyncApp, title: &str, bounds: Bounds<Pixels>) -> bool {
    for _ in 0..10 {
        cx.background_executor()
            .timer(Duration::from_millis(50))
            .await;
        let title = title.to_string();
        let configured = cx.update(move |_| configure_and_hide(&title, bounds));
        if let Ok(true) = configured {
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
fn configure_and_hide(title: &str, bounds: Bounds<Pixels>) -> bool {
    if !popup_window::configure_popup_window(title) {
        return false;
    }
    popup_window::set_window_bounds_by_title(
        title,
        bounds.origin.x.to_f64(),
        bounds.origin.y.to_f64(),
        bounds.size.width.to_f64(),
        bounds.size.height.to_f64(),
    );
    popup_window::disable_window_shadow(title);
    popup_window::hide_window_by_title(title);
    true
}

#[cfg(target_os = "linux")]
async fn rehide_after_settle(cx: &AsyncApp, title: &str) {
    for delay_ms in [150u64, 400] {
        cx.background_executor()
            .timer(Duration::from_millis(delay_ms))
            .await;
        let title = title.to_string();
        let _ = cx.update(move |_| {
            popup_window::hide_window_by_title(&title);
        });
    }
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
