mod actions;
mod controller;
mod entry_store;
mod input;
mod layout;
mod render;
mod search;
mod state;
mod view;

use std::sync::{mpsc, Arc};
use std::time::Duration;

use gpui::*;

use crate::daemon;
use crate::monitor;
use crate::open_window_with_focus;
use crate::platform;
use crate::providers::{apps, files};

use entry_store::EntryStore;
use layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;

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
            w.update(cx, |_, window, cx| {
                #[cfg(target_os = "macos")]
                {
                    let _ = window;
                    cx.hide();
                }
                #[cfg(not(target_os = "macos"))]
                {
                    window.minimize_window();
                }
            })
            .ok();
        });
}

fn close_active_window(cx: &mut App) {
    if let Some(active) = cx.active_window() {
        let _ = active.update(cx, |_, window, _cx| window.remove_window());
    }
}

fn open_launcher(entries: Arc<PreloadedEntries>, cx: &mut App) {
    close_active_window(cx);

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

    let _ = open_window_with_focus(cx, options, move |_window, cx| {
        LauncherView::new(entries.clone(), cx)
    });
    cx.activate(true);
}

fn spawn_command_poll(
    entries: Arc<PreloadedEntries>,
    rx: mpsc::Receiver<daemon::Command>,
    cx: &mut App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            cx.background_spawn(async {
                std::thread::sleep(Duration::from_millis(50));
            })
            .await;

            match rx.try_recv().ok() {
                Some(daemon::Command::Show) => {
                    let entries = entries.clone();
                    cx.update(move |cx| open_launcher(entries.clone(), cx)).ok();
                }
                Some(daemon::Command::Kill) => {
                    cx.update(|cx| cx.quit()).ok();
                    break;
                }
                None => {}
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
        open_keepalive_window(cx);
        spawn_command_poll(entries.clone(), rx, cx);

        if show_immediately {
            open_launcher(entries.clone(), cx);
        }
    });

    daemon::cleanup();
}
