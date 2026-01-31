use gpui::*;
use gpui_test::{
    action_for_modifiers, action_hint, action_label, open_window_with_focus, LaunchAction,
};

actions!(test, [Quit]);

struct ActionView {
    items: Vec<String>,
    selected: usize,
    hint: Option<LaunchAction>,
    focus_handle: FocusHandle,
}

impl ActionView {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: vec![
                "Terminal",
                "Files",
                "Settings",
                "Calculator",
                "Notes",
                "Browser",
                "Music",
                "Editor",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            selected: 0,
            hint: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn update_hint(&mut self, modifiers: &Modifiers) {
        self.hint = action_hint(modifiers.control, modifiers.shift, modifiers.alt);
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }
}

fn action_shortcut(action: LaunchAction) -> &'static str {
    match action {
        LaunchAction::Open => "Enter",
        LaunchAction::Terminal => "Ctrl+Enter",
        LaunchAction::OpenFolder => "Shift+Enter",
        LaunchAction::CopyPath => "Alt+Enter",
    }
}

impl Focusable for ActionView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ActionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hint_text = self
            .hint
            .map(|action| format!("{}: {}", action_shortcut(action), action_label(action)))
            .unwrap_or_default();

        div()
            .id("action-modifiers")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                let modifiers = &event.keystroke.modifiers;
                let prev_hint = this.hint;
                this.update_hint(modifiers);
                if prev_hint != this.hint {
                    cx.notify();
                }

                match event.keystroke.key.as_str() {
                    "up" => {
                        this.move_up();
                        cx.notify();
                    }
                    "down" => {
                        this.move_down();
                        cx.notify();
                    }
                    "enter" => {
                        if let Some(item) = this.items.get(this.selected) {
                            let action = action_for_modifiers(
                                modifiers.control,
                                modifiers.shift,
                                modifiers.alt,
                            );
                            println!("Action: {} on {}", action_label(action), item);
                        }
                    }
                    _ => {}
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _window, cx| {
                let prev_hint = this.hint;
                this.update_hint(&event.keystroke.modifiers);
                if prev_hint != this.hint {
                    cx.notify();
                }
            }))
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .border_b_1()
                    .border_color(rgb(0x45475a))
                    .child(
                        div()
                            .text_color(rgb(0xa6adc8))
                            .text_size(px(14.))
                            .child("Enter: Open"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0xf9e2af))
                            .text_size(px(13.))
                            .child(hint_text),
                    ),
            )
            .children(self.items.iter().enumerate().map(|(i, item)| {
                let is_selected = i == self.selected;
                div()
                    .h(px(32.))
                    .w_full()
                    .flex()
                    .items_center()
                    .px_4()
                    .bg(if is_selected { rgb(0x45475a) } else { rgb(0x1e1e2e) })
                    .child(
                        div()
                            .text_color(if is_selected { rgb(0xcdd6f4) } else { rgb(0xa6adc8) })
                            .text_size(px(14.))
                            .child(item.clone()),
                    )
            }))
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Quit, None)]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let height = 42.0 + (8.0 * 32.0);
        let bounds = Bounds::centered(None, size(px(420.), px(height)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        };

        open_window_with_focus(cx, options, |_window, cx| ActionView::new(cx)).unwrap();
        cx.activate(true);
    });
}
