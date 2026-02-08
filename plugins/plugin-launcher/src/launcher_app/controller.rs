use gpui::{ClipboardItem, Context, KeyDownEvent};

use super::actions;
use super::input::InputEffect;
use super::window_ops::hide_in_context;
use super::LauncherView;

enum ClipboardShortcut {
    Copy,
    Cut,
    Paste,
}

impl LauncherView {
    pub(super) fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let secondary = event.keystroke.modifiers.secondary();

        if self.handle_clipboard_shortcut(key, secondary, cx) {
            return;
        }

        self.store.ensure_filtered(&self.state);
        let result_count = self.store.result_count();
        match self.state.apply_key(event, result_count) {
            InputEffect::Ignore => {}
            InputEffect::Navigate => cx.notify(),
            InputEffect::QueryChanged => {
                self.state.selected = 0;
                cx.notify();
            }
            InputEffect::Launch => self.launch_selected(window),
            InputEffect::Dismiss => hide_in_context(window, cx),
        }
    }

    fn handle_clipboard_shortcut(&mut self, key: &str, secondary: bool, cx: &mut Context<Self>) -> bool {
        let Some(shortcut) = Self::clipboard_shortcut(key, secondary) else {
            return false;
        };
        self.apply_clipboard_shortcut(shortcut, cx);
        true
    }

    fn clipboard_shortcut(key: &str, secondary: bool) -> Option<ClipboardShortcut> {
        if !secondary {
            return None;
        }
        match key {
            "c" => Some(ClipboardShortcut::Copy),
            "x" => Some(ClipboardShortcut::Cut),
            "v" => Some(ClipboardShortcut::Paste),
            _ => None,
        }
    }

    fn apply_clipboard_shortcut(&mut self, shortcut: ClipboardShortcut, cx: &mut Context<Self>) {
        match shortcut {
            ClipboardShortcut::Copy => self.copy_selection(cx),
            ClipboardShortcut::Cut => self.cut_selection(cx),
            ClipboardShortcut::Paste => self.paste_from_clipboard(cx),
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.state.selection_text() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn cut_selection(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.state.cut_selection() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.state.selected = 0;
        cx.notify();
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.state.paste_text(&text) {
            self.state.selected = 0;
            cx.notify();
        }
    }

    fn launch_selected(&mut self, window: &mut gpui::Window) {
        self.store.ensure_filtered(&self.state);
        let Some(scored) = self.store.get(self.state.selected) else {
            return;
        };
        let Some(item) = self.store.item(scored) else {
            return;
        };
        if actions::launch_item(&item) {
            #[cfg(target_os = "macos")]
            {
                window.remove_window();
            }
            #[cfg(not(target_os = "macos"))]
            {
                window.minimize_window();
            }
        }
    }
}
