use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;

use crate::monitor;
use crate::open_window_with_focus;
use crate::platform;

use super::layout::{window_height_for_rows, HEADER_HEIGHT, WINDOW_WIDTH};
use super::{LauncherView, PreloadedEntries, SharedEntries, LAUNCHER_APP_ID};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MonitorKey {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl MonitorKey {
    fn from_bounds(bounds: &Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.to_f64().round() as i32,
            y: bounds.origin.y.to_f64().round() as i32,
            width: bounds.size.width.to_f64().round() as i32,
            height: bounds.size.height.to_f64().round() as i32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum LauncherTarget {
    Default,
    Monitor(MonitorKey),
}

impl LauncherTarget {
    fn from_snapshot(snapshot: Option<&monitor::ActiveMonitor>) -> Self {
        snapshot
            .map(|m| Self::Monitor(MonitorKey::from_bounds(&m.bounds())))
            .unwrap_or(Self::Default)
    }
}

#[derive(Default)]
pub(crate) struct ActiveLaunchers {
    windows: HashMap<LauncherTarget, WindowHandle<LauncherView>>,
}

impl ActiveLaunchers {
    fn existing(&self, target: LauncherTarget) -> Option<WindowHandle<LauncherView>> {
        self.windows.get(&target).cloned()
    }

    fn insert(&mut self, target: LauncherTarget, handle: WindowHandle<LauncherView>) {
        self.windows.insert(target, handle);
    }

    fn remove(&mut self, target: LauncherTarget) {
        self.windows.remove(&target);
    }

    pub(crate) fn handles(&self) -> Vec<WindowHandle<LauncherView>> {
        self.windows.values().cloned().collect()
    }

    fn hide_non_target(&mut self, target: LauncherTarget, cx: &mut App) {
        let handles: Vec<(LauncherTarget, WindowHandle<LauncherView>)> = self
            .windows
            .iter()
            .filter(|(key, _)| **key != target)
            .map(|(key, handle)| (*key, handle.clone()))
            .collect();

        let mut removed = Vec::new();
        for (key, handle) in handles {
            let _ = handle.update(cx, |view, window, _| {
                view.set_showing(false);
                window.remove_window();
            });
            removed.push(key);
        }
        for key in removed {
            self.windows.remove(&key);
        }
    }
}

fn snapshot_entries(entries: &SharedEntries) -> Arc<PreloadedEntries> {
    entries
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| Arc::new(PreloadedEntries::empty()))
}

pub(crate) fn activate_or_open_launcher(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    let target = LauncherTarget::from_snapshot(monitor_snapshot.as_ref());
    let current_entries = snapshot_entries(&entries);
    eprintln!(
        "[launcher] activate_or_open target={target:?} cached_windows={}",
        active.borrow().windows.len()
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
        active.borrow().windows.len()
    );
    active.borrow_mut().hide_non_target(target, cx);

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
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    };

    if let Some(handle) = open_launcher_window(cx, options, current_entries.clone(), target) {
        active.borrow_mut().insert(target, handle);
        eprintln!("[launcher] activate_or_open opened new launcher window");
    } else {
        eprintln!("[launcher] open failed: target={target:?}");
    }

    platform::activate_app(cx);
}

fn open_launcher_window(
    cx: &mut App,
    options: WindowOptions,
    entries: Arc<PreloadedEntries>,
    target: LauncherTarget,
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
    expected_target: LauncherTarget,
    cx: &mut App,
) -> bool {
    let handles: Vec<(LauncherTarget, WindowHandle<LauncherView>)> = active
        .borrow()
        .windows
        .iter()
        .map(|(target, handle)| (*target, *handle))
        .collect();

    let mut visible = None;
    let mut dead = Vec::new();
    for (target, handle) in handles {
        match handle.update(cx, |view, _, _| view.is_showing) {
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
    target: LauncherTarget,
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
            view.store.replace_entries(
                entries.app_entries.clone(),
                entries.file_entries.clone(),
            );
            let should_resize = view.reset_for_show();
            view.store.ensure_filtered(&view.state);
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
