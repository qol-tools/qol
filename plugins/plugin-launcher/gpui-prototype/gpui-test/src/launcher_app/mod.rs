mod actions;
mod input;
mod layout;
mod search;
mod state;
mod view;

use gpui::*;

use crate::desktop_entry::{self, DesktopEntry};
use crate::monitor;
use crate::open_window_with_focus;
use crate::providers::files;

use input::InputEffect;
use layout::{resize_for_visible_rows, HEADER_HEIGHT, MAX_VISIBLE, ROW_HEIGHT, WINDOW_WIDTH};
use state::LauncherState;

pub use input::key_to_input_char;

actions!(launcher, [Quit]);

struct LauncherView {
    state: LauncherState,
    app_entries: Vec<DesktopEntry>,
    file_entries: Vec<files::FileEntry>,
    focus_handle: FocusHandle,
}

impl LauncherView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: LauncherState::new(),
            app_entries: desktop_entry::scan(&desktop_entry::default_dirs()),
            file_entries: files::default_provider().load_entries(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn filtered(&self) -> Vec<search::Scored<'_>> {
        search::filtered(
            &self.app_entries,
            &self.file_entries,
            &self.state.query,
            self.state.mode,
        )
    }

    fn handle_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let secondary = event.keystroke.modifiers.secondary();

        if secondary && key == "c" {
            if let Some(text) = self.state.selection_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return;
        }

        if secondary && key == "x" {
            if let Some(text) = self.state.cut_selection() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.state.selected = 0;
                cx.notify();
            }
            return;
        }

        if secondary && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                if self.state.paste_text(&text) {
                    self.state.selected = 0;
                    cx.notify();
                }
            }
            return;
        }

        let result_count = self.filtered().len();
        match self.state.apply_key(event, result_count) {
            InputEffect::Ignore => {}
            InputEffect::Notify => cx.notify(),
            InputEffect::Launch => {
                let filtered = self.filtered();
                if let Some(scored) = filtered.get(self.state.selected) {
                    actions::launch_item(&scored.item);
                    cx.quit();
                }
            }
        }
    }
}

impl Focusable for LauncherView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let visible = self.filtered().len().min(MAX_VISIBLE);
        let results_height = visible as f32 * ROW_HEIGHT;
        resize_for_visible_rows(&mut self.state.window_height, visible, window);
        let filtered = self.filtered();

        div()
            .id("launcher")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(view::bg_color())
            .on_key_down(cx.listener(Self::handle_key))
            .child(view::search_bar(
                self.state.mode.label(),
                &self.state.query,
                self.state.cursor,
                self.state.selected_range(),
            ))
            .child(
                div()
                    .h(px(results_height))
                    .w_full()
                    .flex()
                    .flex_col()
                    .bg(view::bg_color())
                    .children(
                        filtered
                            .iter()
                            .enumerate()
                            .take(MAX_VISIBLE)
                            .map(|(i, scored)| {
                                view::result_row(scored, i == self.state.selected, ROW_HEIGHT)
                            }),
                    ),
            )
    }
}

pub fn run() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let win_size = size(px(WINDOW_WIDTH), px(HEADER_HEIGHT));
        let bounds = monitor::active(cx)
            .map(|m| m.centered_bounds(win_size))
            .unwrap_or_else(|| Bounds::centered(None, win_size, cx));

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
