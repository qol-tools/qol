mod actions;
mod controller;
mod entry_store;
mod input;
mod layout;
mod render;
mod search;
mod state;
mod view;

use std::sync::mpsc;
use std::time::Duration;

use gpui::*;

use crate::daemon;
use crate::monitor;
use crate::open_window_with_focus;
use crate::platform;

use entry_store::EntryStore;
use layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;

pub use input::key_to_input_char;

actions!(launcher, [Dismiss]);

struct LauncherView {
    pub(super) state: LauncherState,
    pub(super) store: EntryStore,
    pub(super) focus_handle: FocusHandle,
    blur_sub: Option<Subscription>,
}

impl LauncherView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(
                crate::providers::apps::default_provider().load_entries(),
                crate::providers::files::default_provider().load_entries(),
            ),
            focus_handle: cx.focus_handle(),
            blur_sub: None,
        }
    }
}

fn open_launcher(cx: &mut App) {
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

    let _ = open_window_with_focus(cx, options, |_window, cx| LauncherView::new(cx));
    cx.activate(true);
}

fn spawn_command_poll(rx: mpsc::Receiver<daemon::Command>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            cx.background_spawn(async {
                std::thread::sleep(Duration::from_millis(50));
            })
            .await;

            match rx.try_recv().ok() {
                Some(daemon::Command::Show) => {
                    cx.update(|cx| open_launcher(cx)).ok();
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

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Dismiss, None)]);
        cx.on_action(|_: &Dismiss, cx: &mut App| {
            cx.active_window()
                .map(|w| w.update(cx, |_, window, _cx| window.remove_window()).ok());
        });

        spawn_command_poll(rx, cx);

        if show_immediately {
            open_launcher(cx);
        }
    });

    daemon::cleanup();
}
