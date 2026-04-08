use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;

use crate::discovery::SharedEntries;
use crate::monitor;
use crate::open_window_with_focus;

use super::layout::{window_height_for_rows, HEADER_HEIGHT, WINDOW_WIDTH};
use super::platform;
use super::{LauncherView, LAUNCHER_APP_ID};

use qol_plugin_api::window::{
    centered_window_placement, target_monitor_key, ActiveWindows, MonitorKey,
};

pub(crate) type ActiveLaunchers = ActiveWindows<LauncherView>;

fn get_target(snapshot: Option<&monitor::ActiveMonitor>) -> MonitorKey {
    target_monitor_key(snapshot)
}

pub(crate) fn activate_or_open_launcher(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    let target = get_target(monitor_snapshot.as_ref());
    eprintln!(
        "[launcher] activate_or_open target={target:?} cached_windows={}",
        active.borrow().len()
    );
    #[cfg(debug_assertions)]
    eprintln!(
        "[launcher] activate_or_open: snapshot={:?}, target={:?}",
        monitor_snapshot.as_ref().map(|m| m.bounds()),
        target
    );

    if try_activate_visible_launcher(active.clone(), target, cx) {
        eprintln!("[launcher] activate_or_open reused visible launcher");
        return;
    }

    #[cfg(debug_assertions)]
    eprintln!("[launcher] cached_windows={}", active.borrow().len());
    active.borrow_mut().destroy_non_target(target, cx);

    let win_size = size(px(WINDOW_WIDTH), px(HEADER_HEIGHT));
    let caps = platform::current_capabilities();
    let placement = caps
        .can_window_positioning
        .then(|| centered_window_placement(monitor_snapshot.as_ref(), win_size, cx));
    let bounds = placement
        .map(|placement| placement.bounds)
        .unwrap_or_else(|| Bounds::centered(None, win_size, cx));
    let display_id = placement.and_then(|placement| placement.display_id);
    let expected_bounds = caps.can_window_positioning.then_some(bounds);

    if try_activate_existing_launcher(active.clone(), target, expected_bounds, cx) {
        eprintln!("[launcher] activate_or_open reused existing launcher");
        return;
    }

    #[cfg(debug_assertions)]
    eprintln!("[launcher] opening at {:?}", bounds);

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: true,
        is_movable: false,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    };

    match open_launcher_window(cx, options, entries.clone(), target) {
        Some(handle) => {
            active.borrow_mut().insert(target, handle);
            eprintln!("[launcher] activate_or_open opened new launcher window");
        }
        None => eprintln!("[launcher] open failed: target={target:?}"),
    }

    platform::activate_app(cx);
}

fn open_launcher_window(
    cx: &mut App,
    options: WindowOptions,
    shared: SharedEntries,
    target: MonitorKey,
) -> Option<WindowHandle<LauncherView>> {
    match open_window_with_focus(cx, options, {
        let shared = shared.clone();
        move |_window, cx| LauncherView::new(shared.clone(), cx)
    }) {
        Ok(handle) => {
            eprintln!("[launcher] popup open succeeded");
            Some(handle)
        }
        Err(error) => {
            eprintln!(
                "[launcher] popup open failed for target={target:?}: {:?}",
                error
            );
            let fallback_size = size(px(WINDOW_WIDTH), px(HEADER_HEIGHT));
            let fallback_placement = centered_window_placement(None, fallback_size, cx);
            let fallback_bounds = fallback_placement.bounds;
            let fallback_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(fallback_bounds)),
                display_id: fallback_placement.display_id,
                titlebar: None,
                window_decorations: Some(WindowDecorations::Client),
                kind: WindowKind::Normal,
                focus: true,
                is_movable: false,
                app_id: Some(LAUNCHER_APP_ID.to_string()),
                ..Default::default()
            };

            match open_window_with_focus(cx, fallback_options, move |_window, cx| {
                LauncherView::new(shared.clone(), cx)
            }) {
                Ok(handle) => {
                    eprintln!("[launcher] fallback open succeeded");
                    Some(handle)
                }
                Err(fallback_error) => {
                    eprintln!("[launcher] fallback open failed: {:?}", fallback_error);
                    None
                }
            }
        }
    }
}

fn try_activate_visible_launcher(
    active: Rc<RefCell<ActiveLaunchers>>,
    expected_target: MonitorKey,
    cx: &mut App,
) -> bool {
    let handles = active.borrow().iter();

    let mut visible = None;
    let mut dead = Vec::new();
    for (target, handle) in handles {
        match handle.update(cx, |view: &mut LauncherView, _: &mut Window, _| {
            view.is_showing
        }) {
            Ok(true) => {
                visible = Some((target, handle));
                break;
            }
            Ok(false) => {}
            Err(_) => dead.push(target),
        }
    }

    if !dead.is_empty() {
        let mut guard = active.borrow_mut();
        for target in dead {
            guard.remove(target);
        }
    }

    let Some((visible_target, handle)) = visible else {
        return false;
    };

    if visible_target != expected_target {
        return false;
    }

    if activate_launcher_handle(handle, false, cx) {
        return true;
    }

    active.borrow_mut().remove(visible_target);
    false
}

fn try_activate_existing_launcher(
    active: Rc<RefCell<ActiveLaunchers>>,
    target: MonitorKey,
    expected_bounds: Option<Bounds<Pixels>>,
    cx: &mut App,
) -> bool {
    let existing = active.borrow().existing(target);
    let Some(existing) = existing else {
        return false;
    };
    if should_recreate_launcher(&existing, expected_bounds, cx) {
        active.borrow_mut().remove(target);
        return false;
    }

    if !activate_launcher_handle(existing, true, cx) {
        active.borrow_mut().remove(target);
        return false;
    }

    true
}

fn should_recreate_launcher(
    handle: &WindowHandle<LauncherView>,
    expected_bounds: Option<Bounds<Pixels>>,
    cx: &mut App,
) -> bool {
    let Some(expected_bounds) = expected_bounds else {
        return false;
    };
    let Ok(current_bounds) =
        handle.update(cx, |_, window, _| window.window_bounds().get_bounds())
    else {
        return true;
    };
    let origin_dx = (current_bounds.origin.x.to_f64() - expected_bounds.origin.x.to_f64()).abs();
    let origin_dy = (current_bounds.origin.y.to_f64() - expected_bounds.origin.y.to_f64()).abs();
    let width_delta =
        (current_bounds.size.width.to_f64() - expected_bounds.size.width.to_f64()).abs();
    let height_delta =
        (current_bounds.size.height.to_f64() - expected_bounds.size.height.to_f64()).abs();
    origin_dx > 6.0 || origin_dy > 6.0 || width_delta > 1.0 || height_delta > 1.0
}

fn activate_launcher_handle(
    handle: WindowHandle<LauncherView>,
    resize_to_header: bool,
    cx: &mut App,
) -> bool {
    let activated = handle
        .update(cx, |view, window, cx| {
            view.sync_entries_from_shared();
            let should_resize = view.reset_for_show();
            view.store
                .ensure_filtered(&view.state.query, view.state.mode, view.state.fuzziness);
            if resize_to_header && should_resize {
                window.resize(size(px(WINDOW_WIDTH), px(window_height_for_rows(0))));
            }
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            cx.notify();
        })
        .is_ok();

    if activated {
        platform::activate_app(cx);
    }

    activated
}
