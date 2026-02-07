mod actions;
mod controller;
mod entry_store;
mod input;
mod layout;
mod render;
mod search;
mod state;
mod view;

use gpui::*;

use crate::monitor;
use crate::open_window_with_focus;
use crate::platform;
use crate::providers::{apps, files};

use entry_store::EntryStore;
use layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;

pub use input::key_to_input_char;

actions!(launcher, [Quit]);

struct LauncherView {
    pub(super) state: LauncherState,
    pub(super) store: EntryStore,
    pub(super) focus_handle: FocusHandle,
}

impl LauncherView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: LauncherState::new(),
            store: EntryStore::new(
                apps::default_provider().load_entries(),
                files::default_provider().load_entries(),
            ),
            focus_handle: cx.focus_handle(),
        }
    }
}

pub fn run() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

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

        open_window_with_focus(cx, options, |_window, cx| LauncherView::new(cx)).unwrap();
        cx.activate(true);
    });
}
