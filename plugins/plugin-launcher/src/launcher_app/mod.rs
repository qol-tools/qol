mod actions;
mod controller;
mod entry_store;
mod input;
mod layout;
mod render;
mod search;
mod state;
mod view;
mod window_ops;

use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::*;

use crate::daemon;
use crate::monitor::{self, FocusCache};
use crate::open_window_with_focus;
use crate::platform;
use crate::providers::{apps, files};

use entry_store::EntryStore;
use layout::{window_height_for_rows, HEADER_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;
use window_ops::hide_in_app;

pub use input::key_to_input_char;

const BLUR_GUARD_MS: u64 = 180;

struct PreloadedEntries {
    app_entries: Arc<Vec<apps::AppEntry>>,
    file_entries: Arc<Vec<files::FileEntry>>,
}

impl PreloadedEntries {
    fn load() -> Self {
        Self {
            app_entries: Arc::new(crate::providers::apps::default_provider().load_entries()),
            file_entries: Arc::new(crate::providers::files::default_provider().load_entries()),
        }
    }
}

struct KeepAliveView {
    focus_handle: FocusHandle,
}

impl KeepAliveView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Focusable for KeepAliveView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for KeepAliveView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

struct LauncherView {
    pub(super) state: LauncherState,
    pub(super) store: EntryStore,
    pub(super) focus_handle: FocusHandle,
    blur_sub: Option<Subscription>,
    is_showing: bool,
    blur_guard_until: Instant,
}

impl LauncherView {
    fn new(entries: Arc<PreloadedEntries>, cx: &mut Context<Self>) -> Self {
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(entries.app_entries.clone(), entries.file_entries.clone()),
            focus_handle: cx.focus_handle(),
            blur_sub: None,
            is_showing: true,
            blur_guard_until: Instant::now() + Duration::from_millis(BLUR_GUARD_MS),
        }
    }

    fn reset_for_show(&mut self) -> bool {
        let should_resize = (self.state.window_height - HEADER_HEIGHT).abs() > f32::EPSILON;
        self.state = LauncherState::new();
        self.is_showing = true;
        self.blur_guard_until = Instant::now() + Duration::from_millis(BLUR_GUARD_MS);
        should_resize
    }
}

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
            .map(|m| Self::Monitor(MonitorKey::from_bounds(m.bounds())))
            .unwrap_or(Self::Default)
    }
}

#[derive(Default)]
struct ActiveLaunchers {
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
                view.is_showing = false;
                window.remove_window();
            });
            removed.push(key);
        }
        for key in removed {
            self.windows.remove(&key);
        }
    }
}

fn open_keepalive_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1.), px(1.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: false,
        ..Default::default()
    };

    let _ = cx
        .open_window(options, |_window, cx| cx.new(KeepAliveView::new))
        .map(|w| {
            w.update(cx, |_, window, cx| hide_in_app(window, cx)).ok();
        });
}

fn activate_or_open_launcher(
    entries: Arc<PreloadedEntries>,
    active: Rc<RefCell<ActiveLaunchers>>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    if try_activate_visible_launcher(active.clone(), cx) {
        return;
    }

    let target = LauncherTarget::from_snapshot(monitor_snapshot.as_ref());
    active.borrow_mut().hide_non_target(target, cx);

    if try_activate_existing_launcher(active.clone(), target, cx) {
        return;
    }

    let win_size = size(px(WINDOW_WIDTH), px(HEADER_HEIGHT));
    let caps = platform::current_capabilities();
    let bounds = if caps.can_window_positioning {
        monitor_snapshot.as_ref()
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
        ..Default::default()
    };

    if let Ok(handle) = open_window_with_focus(cx, options, move |_window, cx| {
        LauncherView::new(entries.clone(), cx)
    }) {
        active.borrow_mut().insert(target, handle);
    }
    cx.activate(true);
}

fn try_activate_visible_launcher(
    active: Rc<RefCell<ActiveLaunchers>>,
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

    let Some((target, handle)) = visible else {
        return false;
    };

    if activate_launcher_handle(handle, false, cx) {
        return true;
    }

    active.borrow_mut().remove(target);
    false
}

fn try_activate_existing_launcher(
    active: Rc<RefCell<ActiveLaunchers>>,
    target: LauncherTarget,
    cx: &mut App,
) -> bool {
    let existing = active.borrow().existing(target);
    let Some(existing) = existing else {
        return false;
    };

    if !activate_launcher_handle(existing, true, cx) {
        active.borrow_mut().remove(target);
        return false;
    }

    true
}

fn activate_launcher_handle(
    handle: WindowHandle<LauncherView>,
    resize_to_header: bool,
    cx: &mut App,
) -> bool {
    let activated = handle
        .update(cx, |view, window, cx| {
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
        cx.activate(true);
    }

    activated
}

fn spawn_command_poll(
    entries: Arc<PreloadedEntries>,
    active: Rc<RefCell<ActiveLaunchers>>,
    rx: mpsc::Receiver<daemon::Command>,
    cx: &mut App,
) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            let next_command = cx
                .background_spawn({
                    let rx = rx.clone();
                    async move {
                        let guard = rx.lock().ok()?;
                        guard.recv().ok()
                    }
                })
                .await;

            match next_command {
                Some(daemon::Command::Show(snapshot)) => {
                    let entries = entries.clone();
                    let active = active.clone();
                    cx.update(move |cx| activate_or_open_launcher(entries.clone(), active.clone(), snapshot, cx))
                        .ok();
                }
                Some(daemon::Command::Kill) => {
                    cx.update(|cx| cx.quit()).ok();
                    break;
                }
                None => break,
            }
        }
    })
    .detach();
}

pub fn run() {
    let show_immediately = std::env::args().any(|a| a == "--show");

    if std::env::args().any(|a| a == "--kill") {
        daemon::send_kill();
        return;
    }

    let entries = Arc::new(PreloadedEntries::load());

    Application::new().run(move |cx: &mut App| {
        let focus_cache = FocusCache::start(cx);

        let (tx, rx) = mpsc::channel();
        if !daemon::start_listener(tx, focus_cache.clone()) {
            cx.quit();
            return;
        }

        let active: Rc<RefCell<ActiveLaunchers>> = Rc::new(RefCell::new(ActiveLaunchers::default()));

        open_keepalive_window(cx);
        spawn_command_poll(entries.clone(), active.clone(), rx, cx);

        if show_immediately {
            let snapshot = focus_cache.snapshot();
            activate_or_open_launcher(entries.clone(), active.clone(), snapshot, cx);
        }
    });

    daemon::cleanup();
}
