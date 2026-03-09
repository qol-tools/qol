use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;

use crate::discovery::{PreloadedEntries, SharedEntries};
use crate::monitor;
use crate::open_window_with_focus;

use super::layout::{window_height_for_rows, HEADER_HEIGHT, WINDOW_WIDTH};
use super::platform;
use super::{LauncherView, LAUNCHER_APP_ID};

use qol_plugin_api::window::{ActiveWindows, MonitorKey};

pub(crate) type ActiveLaunchers = ActiveWindows<LauncherView>;

fn get_target(snapshot: Option<&monitor::ActiveMonitor>) -> MonitorKey {
    snapshot
        .map(|m| MonitorKey::from_bounds(&m.bounds()))
        .unwrap_or(MonitorKey {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        })
}

fn snapshot_entries(entries: &SharedEntries) -> Arc<PreloadedEntries> {
    entries
        .lock()
        .map(|guard| guard.entries.clone())
        .unwrap_or_else(|_| Arc::new(PreloadedEntries::empty()))
}

pub(crate) fn activate_or_open_launcher(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    let target = get_target(monitor_snapshot.as_ref());
    let current_entries = snapshot_entries(&entries);
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

    if try_activate_visible_launcher(active.clone(), current_entries.clone(), target, cx) {
        eprintln!("[launcher] activate_or_open reused visible launcher");
        return;
    }

    #[cfg(debug_assertions)]
    eprintln!(
        "[launcher] cached_windows={}",
        active.borrow().len()
    );
    active.borrow_mut().destroy_non_target(target, cx);

    if try_activate_existing_launcher(active.clone(), current_entries.clone(), target, cx) {
        eprintln!("[launcher] activate_or_open reused existing launcher");
        return;
    }

    let win_size = size(px(WINDOW_WIDTH), px(HEADER_HEIGHT));
    let caps = platform::current_capabilities();
    let bounds = if caps.can_window_positioning {
        monitor_snapshot
            .as_ref()
            .map(|m| m.centered_bounds(win_size))
            .unwrap_or_else(|| Bounds::centered(None, win_size, cx))
    } else {
        Bounds::centered(None, win_size, cx)
    };

    #[cfg(debug_assertions)]
    eprintln!("[launcher] opening at {:?}", bounds);

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: true,
        is_movable: false,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    };

    match open_launcher_window(cx, options, current_entries.clone(), target) {
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
    entries: Arc<PreloadedEntries>,
    target: MonitorKey,
) -> Option<WindowHandle<LauncherView>> {
    match open_window_with_focus(cx, options, {
        let entries = entries.clone();
        move |_window, cx| LauncherView::new(entries.clone(), cx)
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
            let fallback_bounds = Bounds::centered(None, fallback_size, cx);
            let fallback_options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(fallback_bounds)),
                titlebar: None,
                window_decorations: Some(WindowDecorations::Client),
                kind: WindowKind::Normal,
                focus: true,
                is_movable: false,
                app_id: Some(LAUNCHER_APP_ID.to_string()),
                ..Default::default()
            };

            match open_window_with_focus(cx, fallback_options, move |_window, cx| {
                LauncherView::new(entries.clone(), cx)
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
    entries: Arc<PreloadedEntries>,
    expected_target: MonitorKey,
    cx: &mut App,
) -> bool {
    let handles = active.borrow().iter();

    let mut visible = None;
    let mut dead = Vec::new();
    for (target, handle) in handles {
        match handle.update(cx, |view: &mut LauncherView, _: &mut Window, _| view.is_showing) {
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

    if activate_launcher_handle(handle, false, entries, cx) {
        return true;
    }

    active.borrow_mut().remove(visible_target);
    false
}

fn try_activate_existing_launcher(
    active: Rc<RefCell<ActiveLaunchers>>,
    entries: Arc<PreloadedEntries>,
    target: MonitorKey,
    cx: &mut App,
) -> bool {
    let existing = active.borrow().existing(target);
    let Some(existing) = existing else {
        return false;
    };

    if !activate_launcher_handle(existing, true, entries, cx) {
        active.borrow_mut().remove(target);
        return false;
    }

    true
}

fn activate_launcher_handle(
    handle: WindowHandle<LauncherView>,
    resize_to_header: bool,
    entries: Arc<PreloadedEntries>,
    cx: &mut App,
) -> bool {
    let activated = handle
        .update(cx, |view, window, cx| {
            view.store
                .replace_entries(entries.app_entries.clone(), entries.file_entries.clone());
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
