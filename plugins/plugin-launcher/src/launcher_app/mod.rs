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

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use gpui::*;

use crate::daemon;
use crate::monitor;
use crate::open_window_with_focus;
use crate::platform;
use crate::providers::{apps, files};

use entry_store::EntryStore;
use layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;
use window_ops::hide_in_app;

pub use input::key_to_input_char;

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
}

impl LauncherView {
    fn new(entries: Arc<PreloadedEntries>, cx: &mut Context<Self>) -> Self {
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(entries.app_entries.clone(), entries.file_entries.clone()),
            focus_handle: cx.focus_handle(),
            blur_sub: None,
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
    active: Rc<RefCell<Option<WindowHandle<LauncherView>>>>,
    cx: &mut App,
) {
    if try_activate_existing_launcher(active.clone(), cx) {
        return;
    }

    let win_size = size(px(WINDOW_WIDTH), px(HEADER_HEIGHT));
    let caps = platform::current_capabilities();
    let bounds = if caps.can_window_positioning {
        monitor::active(cx)
            .map(|m| m.centered_bounds(win_size))
            .unwrap_or_else(|| Bounds::centered(None, win_size, cx))
    } else {
        Bounds::centered(None, win_size, cx)
    };

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
        *active.borrow_mut() = Some(handle);
    }
    cx.activate(true);
}

fn try_activate_existing_launcher(
    active: Rc<RefCell<Option<WindowHandle<LauncherView>>>>,
    cx: &mut App,
) -> bool {
    let Some(existing) = active.borrow().as_ref() else {
        return false;
    };

    let activated = existing
        .update(cx, |view, window, cx| {
            view.state.query.clear();
            view.state.cursor = 0;
            view.state.selected = 0;
            view.state.clear_selection();
            view.store.ensure_filtered(&view.state);
            window.activate_window();
            cx.notify();
        })
        .is_ok();

    if !activated {
        active.borrow_mut().take();
        return false;
    }

    cx.activate(true);
    true
}

fn spawn_command_poll(
    entries: Arc<PreloadedEntries>,
    active: Rc<RefCell<Option<WindowHandle<LauncherView>>>>,
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
                Some(daemon::Command::Show) => {
                    let entries = entries.clone();
                    let active = active.clone();
                    cx.update(move |cx| activate_or_open_launcher(entries.clone(), active.clone(), cx))
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

    let (tx, rx) = mpsc::channel();

    if !daemon::start_listener(tx) {
        return;
    }

    let entries = Arc::new(PreloadedEntries::load());

    Application::new().run(move |cx: &mut App| {
        let active: Rc<RefCell<Option<WindowHandle<LauncherView>>>> = Rc::new(RefCell::new(None));

        open_keepalive_window(cx);
        spawn_command_poll(entries.clone(), active.clone(), rx, cx);

        if show_immediately {
            activate_or_open_launcher(entries.clone(), active.clone(), cx);
        }
    });

    daemon::cleanup();
}
