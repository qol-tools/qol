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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::*;

use crate::daemon;
use crate::monitor::{self, MonitorTracker};
use crate::open_window_with_focus;
use crate::platform;
use crate::providers::{apps, files};

use entry_store::EntryStore;
use layout::{window_height_for_rows, HEADER_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;

pub use input::key_to_input_char;

const BLUR_GUARD_MS: u64 = 180;
const TRAIL_DECAY_TICK: Duration = Duration::from_millis(20);
const LAUNCHER_APP_ID: &str = "qol-tray-launcher";

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
    trail_decay_task_running: bool,
    is_showing: bool,
    any_visible: Arc<AtomicBool>,
    blur_guard_until: Instant,
}

impl LauncherView {
    fn new(entries: Arc<PreloadedEntries>, any_visible: Arc<AtomicBool>, cx: &mut Context<Self>) -> Self {
        any_visible.store(true, Ordering::Release);
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(entries.app_entries.clone(), entries.file_entries.clone()),
            focus_handle: cx.focus_handle(),
            blur_sub: None,
            trail_decay_task_running: false,
            is_showing: true,
            any_visible,
            blur_guard_until: Instant::now() + Duration::from_millis(BLUR_GUARD_MS),
        }
    }

    fn set_showing(&mut self, showing: bool) {
        self.is_showing = showing;
        self.any_visible.store(showing, Ordering::Release);
    }

    fn reset_for_show(&mut self) -> bool {
        let should_resize = (self.state.window_height - HEADER_HEIGHT).abs() > f32::EPSILON;
        self.state = LauncherState::new();
        self.trail_decay_task_running = false;
        self.set_showing(true);
        self.blur_guard_until = Instant::now() + Duration::from_millis(BLUR_GUARD_MS);
        should_resize
    }

    fn ensure_trail_decay_tick(&mut self, cx: &mut Context<Self>) {
        if self.trail_decay_task_running || self.state.decayed_momentum() == 0 {
            return;
        }

        self.trail_decay_task_running = true;
        cx.spawn(|this: WeakEntity<LauncherView>, cx: &mut AsyncApp| {
            let async_cx = cx.clone();
            async move {
                Self::trail_decay_loop(this, async_cx).await;
            }
        })
        .detach();
    }

    async fn trail_decay_loop(this: WeakEntity<Self>, mut async_cx: AsyncApp) {
        let mut last_level = u8::MAX;

        loop {
            async_cx.background_executor().timer(TRAIL_DECAY_TICK).await;
            if !Self::run_trail_decay_step(&this, &mut async_cx, &mut last_level) {
                break;
            }
        }
    }

    fn run_trail_decay_step(
        this: &WeakEntity<Self>,
        async_cx: &mut AsyncApp,
        last_level: &mut u8,
    ) -> bool {
        this.update(async_cx, |view, cx| view.apply_trail_decay_update(last_level, cx))
            .unwrap_or(false)
    }

    fn apply_trail_decay_update(&mut self, last_level: &mut u8, cx: &mut Context<Self>) -> bool {
        let level = self.state.decayed_momentum();
        if level == 0 {
            self.state.previous_selected = None;
            self.state.nav_direction = None;
            self.trail_decay_task_running = false;
            cx.notify();
            return false;
        }

        if level == *last_level {
            return true;
        }

        *last_level = level;
        cx.notify();
        true
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

fn open_keepalive_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1.), px(1.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    };

    let _ = cx.open_window(options, |_window, cx| cx.new(KeepAliveView::new));
}

fn activate_or_open_launcher(
    entries: Arc<PreloadedEntries>,
    active: Rc<RefCell<ActiveLaunchers>>,
    any_visible: Arc<AtomicBool>,
    monitor_snapshot: Option<monitor::ActiveMonitor>,
    cx: &mut App,
) {
    let target = LauncherTarget::from_snapshot(monitor_snapshot.as_ref());
    #[cfg(debug_assertions)]
    eprintln!("[launcher] activate_or_open: snapshot={:?}, target={:?}", monitor_snapshot.as_ref().map(|m| m.bounds()), target);

    if try_activate_visible_launcher(active.clone(), target, cx) {
        #[cfg(debug_assertions)]
        eprintln!("[launcher] reused visible launcher on same target");
        return;
    }

    #[cfg(debug_assertions)]
    eprintln!("[launcher] cached_windows={}", active.borrow().windows.len());
    active.borrow_mut().hide_non_target(target, cx);

    if try_activate_existing_launcher(active.clone(), target, cx) {
        #[cfg(debug_assertions)]
        eprintln!("[launcher] reused existing launcher for target");
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
        is_movable: false,
        focus: true,
        app_id: Some(LAUNCHER_APP_ID.to_string()),
        ..Default::default()
    };

    if let Ok(handle) = open_window_with_focus(cx, options, move |_window, cx| {
        LauncherView::new(entries.clone(), any_visible, cx)
    }) {
        active.borrow_mut().insert(target, handle);
    }
    #[cfg(not(target_os = "macos"))]
    cx.activate(true);
}

fn try_activate_visible_launcher(
    active: Rc<RefCell<ActiveLaunchers>>,
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

    // Cursor moved to a different monitor — let the caller re-target.
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

    #[cfg(not(target_os = "macos"))]
    if activated {
        cx.activate(true);
    }

    activated
}

fn spawn_command_poll(
    entries: Arc<PreloadedEntries>,
    active: Rc<RefCell<ActiveLaunchers>>,
    any_visible: Arc<AtomicBool>,
    rx: mpsc::Receiver<daemon::Command>,
    focus_cache: MonitorTracker,
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

            #[cfg(debug_assertions)]
            eprintln!("[launcher] command_poll: next_command={}", match &next_command {
                Some(daemon::Command::Show) => "Show",
                Some(daemon::Command::Kill) => "Kill",
                None => "None",
            });
            match next_command {
                Some(daemon::Command::Show) => {
                    let focus_cache = focus_cache.clone();
                    let snapshot = cx
                        .background_spawn(async move { focus_cache.snapshot() })
                        .await;
                    #[cfg(debug_assertions)]
                    eprintln!("[launcher] command_poll: snapshot={:?}", snapshot.as_ref().map(|m| m.bounds()));
                    let entries = entries.clone();
                    let active = active.clone();
                    let vis = any_visible.clone();
                    if let Err(e) = cx.update(move |cx| activate_or_open_launcher(entries.clone(), active.clone(), vis, snapshot, cx)) {
                        #[cfg(debug_assertions)]
                        eprintln!("[launcher] command_poll: cx.update failed: {:?}", e);
                    }
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

#[cfg(target_os = "macos")]
fn set_macos_accessory_policy() {
    use std::ffi::c_void;

    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const u8) -> *mut c_void;
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(receiver: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }

    const NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY: i64 = 1;

    unsafe {
        let cls = objc_getClass(b"NSApplication\0".as_ptr());
        let shared_app_sel = sel_registerName(b"sharedApplication\0".as_ptr());
        let app = objc_msgSend(cls, shared_app_sel);
        if !app.is_null() {
            let set_policy_sel = sel_registerName(b"setActivationPolicy:\0".as_ptr());
            objc_msgSend(app, set_policy_sel, NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY);
        }
    }
}

pub fn run() {
    let show_immediately = std::env::args().any(|a| a == "--show");

    if std::env::args().any(|a| a == "--kill") {
        daemon::send_kill();
        return;
    }

    if show_immediately && daemon::send_show() {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        #[cfg(debug_assertions)]
        eprintln!("[launcher] run: pid={}", std::process::id());

        #[cfg(target_os = "macos")]
        set_macos_accessory_policy();

        let any_visible = Arc::new(AtomicBool::new(false));
        let focus_cache = MonitorTracker::start(cx, any_visible.clone());

        let (tx, rx) = mpsc::channel();
        if !daemon::start_listener(tx) {
            #[cfg(debug_assertions)]
            eprintln!("[launcher] daemon listener failed, quitting");
            cx.quit();
            return;
        }

        let entries = Arc::new(PreloadedEntries::load());
        let active: Rc<RefCell<ActiveLaunchers>> = Rc::new(RefCell::new(ActiveLaunchers::default()));

        open_keepalive_window(cx);
        spawn_command_poll(entries.clone(), active.clone(), any_visible.clone(), rx, focus_cache.clone(), cx);

        if show_immediately {
            #[cfg(debug_assertions)]
            eprintln!("[launcher] show_immediately");
            let snapshot = focus_cache.snapshot();
            activate_or_open_launcher(entries.clone(), active.clone(), any_visible.clone(), snapshot, cx);
        }
    });

    daemon::cleanup();
}
