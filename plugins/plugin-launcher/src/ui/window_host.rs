use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;

use crate::discovery::SharedEntries;
use crate::monitor::{self, MonitorTracker};
use crate::open_window_with_focus;

use super::layout::{window_height_for_rows, WINDOW_WIDTH};
use super::{LauncherView, LAUNCHER_APP_ID, LAUNCHER_WINDOW_TITLE};

use qol_gpui::popup_window;
use qol_gpui::window::{centered_window_placement, ActiveWindows, MonitorKey, WindowPlacement};

pub(crate) type ActiveLaunchers = ActiveWindows<LauncherView>;

/// Sentinel slot for the single persistent ghost window. Negative dims never
/// collide with a real monitor key.
const GHOST_KEY: MonitorKey = MonitorKey {
    x: i32::MIN,
    y: i32::MIN,
    width: -1,
    height: -1,
};

fn header_size() -> Size<Pixels> {
    size(px(WINDOW_WIDTH), px(window_height_for_rows(0)))
}

/// Open the launcher window once at boot, drawn but invisible at alpha=0. The
/// hotkey path repositions and reveals it; dismiss hides it again. The window
/// is never destroyed, so showing never pays window-creation cost.
pub(crate) fn pre_create_ghost(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    let placement = centered_window_placement(monitor_snapshot.as_ref(), header_size(), cx);
    let Some(handle) = open_hidden_ghost(cx, entries, &placement) else {
        eprintln!("[launcher] pre-create failed; will open on demand");
        return;
    };
    active.borrow_mut().insert(GHOST_KEY, handle);
    let _ = handle.update(cx, |view, _window, _cx| view.set_showing(false));
    popup_window::configure_popup_window(LAUNCHER_WINDOW_TITLE);
    popup_window::hide_window_by_title(LAUNCHER_WINDOW_TITLE);
}

/// Subscribe to monitor changes and keep the hidden ghost parked on the active
/// monitor, so the next show lands in the right place with no visible jump.
pub(crate) fn spawn_ghost_reposition_listener(
    active: Rc<RefCell<ActiveLaunchers>>,
    focus_cache: MonitorTracker,
    cx: &mut App,
) {
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::ActiveMonitorChanged],
        move |app| reposition_idle_ghost(&active, &focus_cache, app),
    );
}

fn reposition_idle_ghost(
    active: &Rc<RefCell<ActiveLaunchers>>,
    focus_cache: &MonitorTracker,
    cx: &mut App,
) {
    let Some((_, handle)) = active.borrow().any_existing() else {
        return;
    };
    let showing = handle
        .update(cx, |view, _, _| view.is_showing)
        .unwrap_or(true);
    if showing {
        return;
    }
    reposition_to_active(focus_cache.snapshot_monitor().as_ref(), cx);
}

pub(crate) fn activate_or_open_launcher(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    crate::config::apply_ghost_debug();
    if show_ghost(active.clone(), monitor_snapshot.as_ref(), cx) {
        return;
    }
    // Pre-create failed or the slot went stale; open a fresh titled window and
    // adopt it as the ghost from here on.
    create_and_show_ghost(entries, active, monitor_snapshot.as_ref(), cx);
}

fn show_ghost(
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<&monitor::ActiveMonitor>,
    cx: &mut App,
) -> bool {
    let Some((key, handle)) = active.borrow().any_existing() else {
        return false;
    };
    reposition_to_active(monitor_snapshot, cx);
    let prepared = handle
        .update(cx, |view, window, cx| {
            view.sync_entries_from_shared();
            view.reset_for_show();
            view.store
                .ensure_filtered(&view.state.query, view.state.mode, view.state.fuzziness);
            let backing = qol_gpui::popup_window::window_backing_scale(LAUNCHER_WINDOW_TITLE);
            qol_gpui::window::resize_or_sync_scale(window, header_size(), backing);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            cx.notify();
        })
        .is_ok();
    if !prepared {
        active.borrow_mut().remove(key);
        return false;
    }
    popup_window::show_window_by_title(LAUNCHER_WINDOW_TITLE);
    cx.activate(true);
    true
}

fn create_and_show_ghost(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<&monitor::ActiveMonitor>,
    cx: &mut App,
) {
    let placement = centered_window_placement(monitor_snapshot, header_size(), cx);
    let Some(handle) = open_visible_ghost(cx, entries, &placement) else {
        eprintln!("[launcher] open failed");
        return;
    };
    active.borrow_mut().insert(GHOST_KEY, handle);
    popup_window::configure_popup_window(LAUNCHER_WINDOW_TITLE);
    cx.activate(true);
}

fn reposition_to_active(monitor_snapshot: Option<&monitor::ActiveMonitor>, cx: &mut App) {
    let placement = centered_window_placement(monitor_snapshot, header_size(), cx);
    popup_window::reposition_window_by_title(
        LAUNCHER_WINDOW_TITLE,
        placement.bounds.origin.x.to_f64(),
        placement.bounds.origin.y.to_f64(),
    );
}

fn open_hidden_ghost(
    cx: &mut App,
    entries: SharedEntries,
    placement: &WindowPlacement,
) -> Option<WindowHandle<LauncherView>> {
    cx.open_window(ghost_window_options(placement, false), move |window, cx| {
        window.set_window_title(LAUNCHER_WINDOW_TITLE);
        cx.new(move |cx| LauncherView::new(entries, cx))
    })
    .ok()
}

fn open_visible_ghost(
    cx: &mut App,
    entries: SharedEntries,
    placement: &WindowPlacement,
) -> Option<WindowHandle<LauncherView>> {
    open_window_with_focus(
        cx,
        ghost_window_options(placement, true),
        move |window, cx| {
            window.set_window_title(LAUNCHER_WINDOW_TITLE);
            LauncherView::new(entries, cx)
        },
    )
    .ok()
}

fn ghost_window_options(placement: &WindowPlacement, focus: bool) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(placement.bounds)),
        display_id: placement.display_id,
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus,
        is_movable: false,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    }
}
