use gpui::prelude::*;
use gpui::{div, px, rgb, rgba, App, ClickEvent, CursorStyle, KeyDownEvent, Window};

use crate::theme::{SystemPalette, DARK_REFERENCE};

const BUTTON_SIZE: f32 = 24.0;
const BUTTON_GLYPH_SIZE: f32 = 13.0;

const fn button_defaults() -> (u32, u32, u32) {
    let system = SystemPalette::from_reference(DARK_REFERENCE);
    (system.text_muted, 0xffffff0f, system.accent)
}

type IconAction = Box<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct IconButton {
    id: &'static str,
    glyph: &'static str,
    text: u32,
    hover: u32,
    focus: u32,
    action: Option<IconAction>,
}

impl IconButton {
    pub fn new(id: &'static str, glyph: &'static str) -> Self {
        let (text, hover, focus) = button_defaults();
        Self {
            id,
            glyph,
            text,
            hover,
            focus,
            action: None,
        }
    }

    pub fn style(mut self, text: u32, hover: u32, focus: u32) -> Self {
        self.text = text;
        self.hover = hover;
        self.focus = focus;
        self
    }

    pub fn on_activate(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.action = Some(Box::new(action));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let action = std::rc::Rc::new(self.action);
        let click_action = std::rc::Rc::clone(&action);
        div()
            .id(self.id)
            .focusable()
            .tab_stop(true)
            .w(px(BUTTON_SIZE))
            .h(px(BUTTON_SIZE))
            .rounded_md()
            .border_1()
            .border_color(rgba(0))
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(self.text))
            .text_size(px(BUTTON_GLYPH_SIZE))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgba(self.hover)))
            .in_focus(|style| style.border_color(rgb(self.focus)))
            .on_click(move |event, window, app| {
                if accepts_activation_click(event) {
                    if let Some(action) = &*click_action {
                        action(window, app);
                    }
                    app.stop_propagation();
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, app| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") && !event.is_held {
                    if let Some(action) = &*action {
                        action(window, app);
                        app.stop_propagation();
                    }
                }
            })
            .child(self.glyph)
    }
}

pub fn accepts_activation_click(event: &ClickEvent) -> bool {
    matches!(event, ClickEvent::Mouse(_))
}

#[cfg(test)]
mod tests {
    use super::accepts_activation_click;

    #[test]
    fn only_mouse_clicks_activate_icon_buttons() {
        assert!(accepts_activation_click(&gpui::ClickEvent::Mouse(
            Default::default()
        )));
        assert!(!accepts_activation_click(&gpui::ClickEvent::Keyboard(
            gpui::KeyboardClickEvent {
                button: gpui::KeyboardButton::Enter,
                bounds: Default::default(),
            }
        )));
    }
}
