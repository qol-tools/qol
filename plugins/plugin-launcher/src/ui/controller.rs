use gpui::{ClipboardItem, Context, KeyDownEvent};

use super::input::InputEffect;
use super::trace;
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
        let control = event.keystroke.modifiers.control;
        let shift = event.keystroke.modifiers.shift;
        let alt = event.keystroke.modifiers.alt;

        if self.handle_clipboard_shortcut(key, secondary, cx) {
            return;
        }
        let result_count = self.store.result_count();
        let effect = self
            .state
            .apply_key(key, secondary, control, shift, alt, result_count);
        trace::input(self, key, effect, result_count);

        if !matches!(effect, InputEffect::BoostUp | InputEffect::BoostDown) {
            self.state.boost_adjusting = false;
        }

        match effect {
            InputEffect::Ignore => {}
            InputEffect::Navigate => {
                self.store.ensure_filtered(
                    &self.state.query,
                    self.state.mode,
                    self.state.fuzziness,
                );
                self.state.sync_result_window(self.store.result_count());
                cx.notify();
            }
            InputEffect::QueryChanged => {
                self.state.reset_results_position();
                self.schedule_query_render(cx);
            }
            InputEffect::BoostUp | InputEffect::BoostDown => {
                let delta = if matches!(effect, InputEffect::BoostUp) {
                    25
                } else {
                    -25
                };
                self.state.boost_adjusting = true;
                self.adjust_selected_boost(delta);
                self.store.invalidate_cache();
                self.store.ensure_filtered(
                    &self.state.query,
                    self.state.mode,
                    self.state.fuzziness,
                );
                cx.notify();
            }
            InputEffect::Launch => self.launch_selected(window, cx),
            InputEffect::Dismiss => self.hide_to_ghost("key"),
        }
    }

    fn handle_clipboard_shortcut(
        &mut self,
        key: &str,
        secondary: bool,
        cx: &mut Context<Self>,
    ) -> bool {
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
        self.state.reset_results_position();
        cx.notify();
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if self.state.paste_text(&text) {
            self.state.reset_results_position();
            cx.notify();
        }
    }

    fn adjust_selected_boost(&mut self, delta: i32) {
        let Some(scored) = self.store.get(self.state.selected) else {
            return;
        };
        if !matches!(scored.source, crate::discovery::search::ResultSource::App) {
            return;
        }
        let name = self.store.name(scored).to_string();
        self.store.adjust_boost(&name, delta);
    }

    fn launch_selected(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) {
        self.store
            .ensure_filtered(&self.state.query, self.state.mode, self.state.fuzziness);
        let Some(scored) = self.store.get(self.state.selected) else {
            eprintln!(
                "[controller] launch_selected: no scored item at index {}",
                self.state.selected
            );
            return;
        };
        eprintln!(
            "[controller] launch_selected: index={} source={:?} name={:?}",
            self.state.selected,
            scored.source,
            self.store.name(scored)
        );
        let Some(item) = self.store.item(scored) else {
            eprintln!("[controller] launch_selected: failed to resolve item");
            return;
        };
        let is_app = matches!(scored.source, crate::discovery::search::ResultSource::App);
        let name = self.store.name(scored).to_string();
        eprintln!("[controller] launching item...");
        if !crate::launch::launch_item(&item) {
            eprintln!("[controller] launch returned false");
            return;
        }
        eprintln!("[controller] launch succeeded, hiding window");
        if is_app {
            self.store.record_launch(&name);
        }
        self.hide_to_ghost("launch");
    }
}
