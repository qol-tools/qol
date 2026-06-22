use std::cell::RefCell;
use std::rc::Rc;

use gpui::*;

use crate::discovery::SharedEntries;
use crate::monitor::{self, MonitorTracker};
use crate::open_window_with_focus;

use super::layout::{window_height_for_rows, WINDOW_WIDTH};
use super::{trace, LauncherView, LAUNCHER_APP_ID, LAUNCHER_WINDOW_TITLE};

use qol_gpui::popup_window;
use qol_gpui::window::{centered_window_placement, ActiveWindows, MonitorKey, WindowPlacement};

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

fn non_target_keys(keys: &[MonitorKey], target: MonitorKey) -> Vec<MonitorKey> {
    keys.iter().copied().filter(|&key| key != target).collect()
}

fn mark_non_target_hidden(active: &Rc<RefCell<ActiveLaunchers>>, target: MonitorKey, cx: &mut App) {
    let keys: Vec<MonitorKey> = active
        .borrow()
        .iter()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    let mut stale = Vec::new();

    for key in non_target_keys(&keys, target) {
        let Some(handle) = active.borrow().existing(key) else {
            continue;
        };
        if handle
            .update(cx, |view, _window, _cx| view.set_showing(false))
            .is_err()
        {
            stale.push(key);
        }
    }

    if stale.is_empty() {
        return;
    }

    let mut active = active.borrow_mut();
    for key in stale {
        active.remove(key);
    }
}

fn show_ghost(
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<&monitor::ActiveMonitor>,
    cx: &mut App,
) -> bool {
    let _reason = qol_gpui::popup_window::reason_scope("show");
    let placement = centered_window_placement(monitor_snapshot, header_size(), cx);
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
            view.store
                .ensure_filtered(&view.state.query, view.state.mode, view.state.fuzziness);
            qol_gpui::ghost::sync_window_layout(
                &title,
                window,
                placement.bounds.origin,
                header_size(),
            );
            qol_gpui::ghost::show_ghost_window(&title, &all_titles);
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
    cx: &mut App,
) {
    let _reason = qol_gpui::popup_window::reason_scope("create");
    let placement = centered_window_placement(monitor_snapshot, header_size(), cx);
    let target = placement.target;
    let title = qol_gpui::ghost::ghost_window_title(LAUNCHER_WINDOW_TITLE, target);

    mark_non_target_hidden(&active, target, cx);
    let Some(handle) = open_visible_ghost(cx, entries, &placement, &title) else {
        eprintln!("[launcher] open failed");
        return;
    };
    active.borrow_mut().insert(target, handle);
    let all_titles = active.borrow().titles(LAUNCHER_WINDOW_TITLE);
    popup_window::configure_popup_window(&title);
    let _ = handle.update(cx, |view, window, cx| {
        view.set_showing(true);
        qol_gpui::ghost::show_ghost_window(&title, &all_titles);
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
        .open_window(ghost_window_options(placement, false), {
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
    let origin = placement.bounds.origin;
    open_window_with_focus(
        cx,
        ghost_window_options(placement, true),
        move |window, cx| {
            window.set_window_title(&title);
            let mut view = LauncherView::new(title.clone(), entries, cx);
            view.set_window_origin(origin);
            view
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

#[cfg(test)]
mod tests {
    use super::{non_target_keys, MonitorKey};
    use proptest::prelude::*;

    fn key(x: i32) -> MonitorKey {
        MonitorKey {
            x,
            y: 0,
            width: 100,
            height: 100,
        }
    }

    #[test]
    fn hides_every_ghost_except_the_target() {
        let keys = [key(0), key(1), key(2)];
        assert_eq!(non_target_keys(&keys, key(1)), vec![key(0), key(2)]);
    }

    #[test]
    fn a_lone_target_hides_nothing() {
        assert!(non_target_keys(&[key(5)], key(5)).is_empty());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn open_leaves_exactly_the_target_showing(
            xs in prop::collection::hash_set(any::<i32>(), 1..8),
            pick in any::<prop::sample::Index>(),
        ) {
            let keys: Vec<MonitorKey> = xs.into_iter().map(key).collect();
            let target = keys[pick.index(keys.len())];

            let hidden = non_target_keys(&keys, target);

            prop_assert!(!hidden.contains(&target));
            let showing: Vec<MonitorKey> =
                keys.iter().copied().filter(|k| !hidden.contains(k)).collect();
            prop_assert_eq!(showing, vec![target]);
        }

        #[test]
        fn opening_a_fresh_monitor_hides_all_existing(
            xs in prop::collection::hash_set(any::<i32>(), 0..8),
            target_x in any::<i32>(),
        ) {
            prop_assume!(!xs.contains(&target_x));
            let keys: Vec<MonitorKey> = xs.into_iter().map(key).collect();
            prop_assert_eq!(non_target_keys(&keys, key(target_x)).len(), keys.len());
        }
    }
}
