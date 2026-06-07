use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;

use crate::discovery::SharedEntries;
use crate::monitor::{self, MonitorTracker};
use crate::open_window_with_focus;

use super::layout::{window_height_for_rows, WINDOW_WIDTH};
use super::{LauncherView, LAUNCHER_APP_ID, LAUNCHER_WINDOW_TITLE};

use qol_gpui::popup_window;
use qol_gpui::window::{centered_window_placement, ActiveWindows, WindowPlacement};

pub(crate) type ActiveLaunchers = ActiveWindows<LauncherView>;

fn header_size() -> Size<Pixels> {
    size(px(WINDOW_WIDTH), px(window_height_for_rows(0)))
}

pub(crate) fn pre_create_ghost(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    tracker: MonitorTracker,
    cx: &mut App,
) {
    crate::config::apply_ghost_debug();
    let monitors = tracker.all_monitors();
    let monitors = if monitors.is_empty() {
        if let Some(m) = tracker.snapshot_monitor() {
            vec![m]
        } else {
            vec![]
        }
    } else {
        monitors
    };
    for monitor in monitors {
        let placement = centered_window_placement(Some(&monitor), header_size(), cx);
        let target = placement.target;
        let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);
        let Some(handle) = open_hidden_ghost(cx, entries.clone(), &placement, &title) else {
            eprintln!(
                "[launcher] pre-create failed for monitor={:?}; will open on demand",
                target
            );
            continue;
        };
        active.borrow_mut().insert(target, handle);
        let _ = handle.update(cx, |view, _window, _cx| view.set_showing(false));

        popup_window::configure_popup_window(&title);
        qol_gpui::ghost::hide_invisible(&title);
    }
    let keys: Vec<_> = active
        .borrow()
        .iter()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    qol_gpui::ghost::reconcile_active(&keys, |key| {
        qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, key)
    });
}

pub(crate) fn spawn_ghost_reposition_listener(
    active: Rc<RefCell<ActiveLaunchers>>,
    focus_cache: MonitorTracker,
    cx: &mut App,
) {
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::ActiveMonitorChanged],
        move |app, event| reposition_idle_ghost(&active, &focus_cache, event, app),
    );
}

pub(crate) fn spawn_topology_listener(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    tracker: MonitorTracker,
    cx: &mut App,
) {
    qol_gpui::event_router::spawn_runtime_event_router(
        cx,
        vec![qol_gpui::protocol::RuntimeEventKind::MonitorsChanged],
        move |app, event| rebuild_ghosts_for_topology(&entries, &active, &tracker, event, app),
    );
}

pub(crate) fn any_showing(active: &Rc<RefCell<ActiveLaunchers>>, cx: &App) -> bool {
    active
        .borrow()
        .iter()
        .into_iter()
        .any(|(_, handle)| handle.read(cx).map(|view| view.is_showing).unwrap_or(false))
}

fn rebuild_ghosts_for_topology(
    entries: &SharedEntries,
    active: &Rc<RefCell<ActiveLaunchers>>,
    tracker: &MonitorTracker,
    event: &qol_gpui::protocol::RuntimeEvent,
    cx: &mut App,
) {
    if !matches!(
        event,
        qol_gpui::protocol::RuntimeEvent::MonitorsChanged { .. }
    ) {
        return;
    }
    if any_showing(active, cx) {
        return;
    }
    qol_gpui::ghost::refresh_active_monitor_from_state();
    let mut stale = std::mem::take(&mut *active.borrow_mut());
    stale.destroy_all(cx);
    pre_create_ghost(entries.clone(), active.clone(), tracker.clone(), cx);
}

fn reposition_idle_ghost(
    active: &Rc<RefCell<ActiveLaunchers>>,
    focus_cache: &MonitorTracker,
    event: &qol_gpui::protocol::RuntimeEvent,
    cx: &App,
) {
    #[cfg(debug_assertions)]
    if let qol_gpui::protocol::RuntimeEvent::ActiveMonitorChanged { monitor_idx, .. } = event {
        qol_runtime::probe!("PLUGIN_RECV_AMC", "monitor_idx={:?}", monitor_idx);
    }
    qol_gpui::ghost::record_active_monitor(event);
    if any_showing(active, cx) {
        return;
    }
    qol_gpui::ghost::reconcile_from_event(
        event,
        &active.borrow(),
        |key| qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, key),
        || focus_cache.snapshot_monitor(),
    );
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
    create_and_show_ghost(entries, active, monitor_snapshot.as_ref(), cx);
}

fn show_ghost(
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<&monitor::ActiveMonitor>,
    cx: &mut App,
) -> bool {
    let placement = centered_window_placement(monitor_snapshot, header_size(), cx);
    let target = placement.target;
    let handle = active.borrow().existing(target);
    let Some(handle) = handle else {
        return false;
    };
    let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);
    let all_titles = active.borrow().titles(LAUNCHER_WINDOW_TITLE);

    let prepared = handle
        .update(cx, |view, window, cx| {
            view.sync_entries_from_shared();
            view.reset_for_show();
            view.store
                .ensure_filtered(&view.state.query, view.state.mode, view.state.fuzziness);
            let backing = qol_gpui::popup_window::window_backing_scale(&title);
            qol_gpui::window::resize_or_sync_scale(window, header_size(), backing);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            cx.notify();
        })
        .is_ok();
    if !prepared {
        active.borrow_mut().remove(target);
        return false;
    }
    qol_gpui::ghost::show_ghost_window(&title, &all_titles);
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
    let target = placement.target;
    let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);
    let Some(handle) = open_visible_ghost(cx, entries, &placement, &title) else {
        eprintln!("[launcher] open failed");
        return;
    };
    active.borrow_mut().insert(target, handle);
    popup_window::configure_popup_window(&title);
    cx.activate(true);
}

fn open_hidden_ghost(
    cx: &mut App,
    entries: SharedEntries,
    placement: &WindowPlacement,
    title: &str,
) -> Option<WindowHandle<LauncherView>> {
    let title = title.to_string();
    let handle = cx
        .open_window(ghost_window_options(placement, false), {
            let title = title.clone();
            move |window, cx| {
                window.set_window_title(&title);
                cx.new(move |cx| LauncherView::new(title.clone(), entries, cx))
            }
        })
        .ok()?;
    qol_gpui::ghost::hide_invisible(&title);
    Some(handle)
}

fn open_visible_ghost(
    cx: &mut App,
    entries: SharedEntries,
    placement: &WindowPlacement,
    title: &str,
) -> Option<WindowHandle<LauncherView>> {
    let title = title.to_string();
    open_window_with_focus(
        cx,
        ghost_window_options(placement, true),
        move |window, cx| {
            window.set_window_title(&title);
            LauncherView::new(title.clone(), entries, cx)
        },
    )
    .ok()
}

fn ghost_window_options(placement: &WindowPlacement, focus: bool) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(placement.bounds)),
        display_id: placement.display_id,
        titlebar: None,
        window_decorations: Some(qol_gpui::platform::ghost_window_decorations(false)),
        kind: qol_gpui::platform::ghost_window_kind(),
        focus,
        is_movable: true,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    }
}
