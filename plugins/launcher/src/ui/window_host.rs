use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;

use crate::discovery::SharedEntries;
use crate::monitor::{self, MonitorTracker};
use crate::open_window_with_focus;

use super::layout::{full_window_height, header_window_height, WINDOW_WIDTH};
use super::{trace, window_memory, LauncherView, LAUNCHER_APP_ID, LAUNCHER_WINDOW_TITLE};

use qol_gpui::popup_window;
use qol_gpui::window::{centered_window_placement, ActiveWindows, MonitorKey, WindowPlacement};
use qol_gpui::window_options::PopupWindowOptions;

pub(crate) type ActiveLaunchers = ActiveWindows<LauncherView>;

fn full_window_size() -> Size<Pixels> {
    size(px(WINDOW_WIDTH), px(full_window_height()))
}

fn native_size(user: Option<Size<Pixels>>, flow: bool) -> Size<Pixels> {
    user.filter(|_| !flow).unwrap_or_else(full_window_size)
}

fn launcher_placement(
    monitor: Option<&monitor::ActiveMonitor>,
    native: Size<Pixels>,
    cx: &App,
) -> WindowPlacement {
    let header = size(native.width, px(header_window_height()));
    let mut placement = centered_window_placement(monitor, header, cx);
    if let Some(area) = monitor.map(monitor::ActiveMonitor::logical_work_area) {
        let lowest = area.origin.y + area.size.height - native.height;
        placement.bounds.origin.y = placement.bounds.origin.y.min(lowest).max(area.origin.y);
    }
    placement
}

pub(crate) fn pre_create_ghost(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    tracker: MonitorTracker,
    cx: &mut App,
) {
    apply_ghost_debug();
    for monitor in tracker.all_monitors_or_snapshot() {
        let placement = launcher_placement(Some(&monitor), full_window_size(), cx);
        let target = placement.target;
        let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);
        let Some(handle) = open_hidden_ghost(cx, entries.clone(), &placement, &title) else {
            log::warn!(
                "pre-create failed for monitor={:?}; will open on demand",
                target
            );
            continue;
        };
        active.borrow_mut().insert(target, handle);
        let _ = handle.update(cx, |view, _window, _cx| view.set_showing(false));

        popup_window::configure_popup_window(&title);
        popup_window::disable_window_shadow(&title);
        popup_window::join_all_spaces(&title);
        qol_gpui::ghost::hide_invisible(&title);
    }
    let keys = active.borrow().keys();
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
    let visible = any_showing(active, cx);
    qol_gpui::ghost::rebuild_on_topology(event, visible, active, cx, |cx| {
        pre_create_ghost(entries.clone(), active.clone(), tracker.clone(), cx)
    });
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
    qol_gpui::monitor::record_active_monitor(event);
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
    flow: Option<String>,
    cx: &mut App,
) {
    apply_ghost_debug();
    if show_ghost(
        active.clone(),
        monitor_snapshot.as_ref(),
        flow.as_deref(),
        cx,
    ) {
        return;
    }
    create_and_show_ghost(
        entries,
        active,
        monitor_snapshot.as_ref(),
        flow.as_deref(),
        cx,
    );
}

fn mark_non_target_hidden(active: &Rc<RefCell<ActiveLaunchers>>, target: MonitorKey, cx: &mut App) {
    qol_gpui::window::hide_non_target(active, target, cx, |view, _window, _cx| {
        view.set_showing(false)
    });
}

fn show_ghost(
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<&monitor::ActiveMonitor>,
    flow: Option<&str>,
    cx: &mut App,
) -> bool {
    let _reason = qol_gpui::popup_window::reason_scope("show");
    let user = window_memory::user_size(monitor_snapshot);
    let native = native_size(user, flow.is_some());
    let placement = launcher_placement(monitor_snapshot, native, cx);
    let target = placement.target;
    let handle = active.borrow().existing(target);
    let Some(handle) = handle else {
        return false;
    };
    let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);

    mark_non_target_hidden(&active, target, cx);
    let all_titles = active.borrow().titles(LAUNCHER_WINDOW_TITLE);

    let prepared = handle
        .update(cx, |view, window, cx| {
            view.sync_entries_from_shared();
            view.reset_for_show();
            view.set_window_origin(placement.bounds.origin);
            view.set_window_sizes(user, native);
            view.store.ensure_filtered(
                view.state.query.text(),
                view.state.mode,
                view.state.fuzziness,
            );
            if let Some(plugin_id) = flow {
                view.open_flow_for(plugin_id, cx);
            }
            qol_gpui::ghost::sync_window_layout(
                &title,
                window,
                placement.bounds.origin,
                native,
                monitor_snapshot.map(monitor::ActiveMonitor::scale),
            );
            super::platform::show_topmost_window(&title, &all_titles);
            window.activate_window();
            window.focus(&view.focus_handle(cx));
            cx.notify();
        })
        .is_ok();
    if !prepared {
        active.borrow_mut().remove(target);
        return false;
    }
    trace::show("reuse", &title, &placement);
    cx.activate(true);
    true
}

fn create_and_show_ghost(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<&monitor::ActiveMonitor>,
    flow: Option<&str>,
    cx: &mut App,
) {
    let _reason = qol_gpui::popup_window::reason_scope("create");
    let user = window_memory::user_size(monitor_snapshot);
    let native = native_size(user, flow.is_some());
    let placement = launcher_placement(monitor_snapshot, native, cx);
    let target = placement.target;
    let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);

    mark_non_target_hidden(&active, target, cx);
    let Some(handle) = open_visible_ghost(cx, entries, &placement, native, &title) else {
        log::warn!("open failed");
        return;
    };
    active.borrow_mut().insert(target, handle);
    let all_titles = active.borrow().titles(LAUNCHER_WINDOW_TITLE);
    popup_window::configure_popup_window(&title);
    popup_window::disable_window_shadow(&title);
    popup_window::join_all_spaces(&title);
    let _ = handle.update(cx, |view, window, cx| {
        view.set_showing(true);
        view.set_window_sizes(user, native);
        if let Some(plugin_id) = flow {
            view.open_flow_for(plugin_id, cx);
        }
        super::platform::show_topmost_window(&title, &all_titles);
        window.activate_window();
        window.focus(&view.focus_handle(cx));
        cx.notify();
    });
    trace::show("create", &title, &placement);
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
        .open_window(
            ghost_window_options(placement, full_window_size(), false),
            {
                let title = title.clone();
                let origin = placement.bounds.origin;
                move |window, cx| {
                    window.set_window_title(&title);
                    cx.new(move |cx| {
                        let mut view = LauncherView::new(title.clone(), entries, cx);
                        view.set_window_origin(origin);
                        view
                    })
                }
            },
        )
        .ok()?;
    qol_gpui::ghost::hide_invisible(&title);
    Some(handle)
}

fn open_visible_ghost(
    cx: &mut App,
    entries: SharedEntries,
    placement: &WindowPlacement,
    native: Size<Pixels>,
    title: &str,
) -> Option<WindowHandle<LauncherView>> {
    let title = title.to_string();
    let origin = placement.bounds.origin;
    open_window_with_focus(
        cx,
        ghost_window_options(placement, native, true),
        move |window, cx| {
            window.set_window_title(&title);
            let mut view = LauncherView::new(title.clone(), entries, cx);
            view.set_window_origin(origin);
            view
        },
    )
    .ok()
}

fn ghost_window_options(
    placement: &WindowPlacement,
    native: Size<Pixels>,
    focus: bool,
) -> WindowOptions {
    let bounds = Bounds {
        origin: placement.bounds.origin,
        size: native,
    };
    PopupWindowOptions::new()
        .bounds(bounds)
        .display_id(placement.display_id)
        .focus(focus)
        .min_size(window_memory::min_size())
        .app_id(LAUNCHER_APP_ID)
        .build()
}

pub(super) fn apply_ghost_debug() {
    #[cfg(debug_assertions)]
    {
        let config = crate::config::load_launcher_config();
        popup_window::set_ghost_debug(
            config.display.ghost_opacity,
            config.display.ghost_debug_color.as_deref(),
        );
    }
}
