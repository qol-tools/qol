use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, App, ClickEvent, CursorStyle, FontWeight, KeyDownEvent,
    MouseButton, SharedString, Window,
};

use crate::surface::PanelDragArea;

const BAR_HEIGHT: f32 = 34.0;
const BAR_PADDING_X: f32 = 12.0;
const TRAILING_GAP: f32 = 9.0;
const TITLE_SIZE: f32 = 11.0;
const BUTTON_SIZE: f32 = 24.0;
const BUTTON_GLYPH_SIZE: f32 = 13.0;
const DEFAULT_BACKGROUND: u32 = 0x14181f;
const DEFAULT_BORDER: u32 = 0x2f3644;
const DEFAULT_TITLE_COLOR: u32 = 0xd4dbea;
const DEFAULT_BUTTON_TEXT: u32 = 0x67748f;
const DEFAULT_BUTTON_HOVER: u32 = 0x2f3644;
const DEFAULT_BUTTON_FOCUS: u32 = 0x8a93a8;

type BarAction = Box<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct WindowBar {
    title: SharedString,
    background: u32,
    border: u32,
    title_color: u32,
    button_text: u32,
    button_hover: u32,
    button_focus: u32,
    collapse: Option<BarAction>,
    hide: Option<BarAction>,
    children: Vec<AnyElement>,
}

impl WindowBar {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            background: DEFAULT_BACKGROUND,
            border: DEFAULT_BORDER,
            title_color: DEFAULT_TITLE_COLOR,
            button_text: DEFAULT_BUTTON_TEXT,
            button_hover: DEFAULT_BUTTON_HOVER,
            button_focus: DEFAULT_BUTTON_FOCUS,
            collapse: None,
            hide: None,
            children: Vec::new(),
        }
    }

    pub fn background(mut self, color: u32) -> Self {
        self.background = color;
        self
    }

    pub fn border(mut self, color: u32) -> Self {
        self.border = color;
        self
    }

    pub fn title_color(mut self, color: u32) -> Self {
        self.title_color = color;
        self
    }

    pub fn button_style(mut self, text: u32, hover: u32, focus: u32) -> Self {
        self.button_text = text;
        self.button_hover = hover;
        self.button_focus = focus;
        self
    }

    pub fn on_collapse(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.collapse = Some(Box::new(action));
        self
    }

    pub fn on_hide(mut self, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.hide = Some(Box::new(action));
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

impl RenderOnce for WindowBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut trailing = div()
            .flex()
            .items_center()
            .gap(px(TRAILING_GAP))
            .children(self.children);
        if let Some(action) = self.collapse {
            trailing = trailing.child(control_button(
                "window-bar-collapse",
                "\u{2581}",
                action,
                self.button_text,
                self.button_hover,
                self.button_focus,
            ));
        }
        if let Some(action) = self.hide {
            trailing = trailing.child(control_button(
                "window-bar-hide",
                "\u{00D7}",
                action,
                self.button_text,
                self.button_hover,
                self.button_focus,
            ));
        }
        div()
            .id("window-bar")
            .h(px(BAR_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px(px(BAR_PADDING_X))
            .bg(rgb(self.background))
            .border_b_1()
            .border_color(rgb(self.border))
            .cursor(CursorStyle::OpenHand)
            .panel_drag_area()
            .on_click(|event, window, _| {
                if is_double_click(event.click_count()) {
                    window.toggle_fullscreen();
                }
            })
            .child(
                div()
                    .text_color(rgb(self.title_color))
                    .text_size(px(TITLE_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title),
            )
            .child(trailing)
    }
}

fn control_button(
    id: &'static str,
    glyph: &'static str,
    action: BarAction,
    text: u32,
    hover: u32,
    focus: u32,
) -> impl IntoElement {
    let action = std::rc::Rc::new(action);
    let click_action = std::rc::Rc::clone(&action);
    div()
        .id(id)
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
        .text_color(rgb(text))
        .text_size(px(BUTTON_GLYPH_SIZE))
        .cursor(CursorStyle::PointingHand)
        .hover(|style| style.bg(rgba(hover)))
        .in_focus(|style| style.border_color(rgb(focus)))
        .on_mouse_down(MouseButton::Left, |_, _, app| app.stop_propagation())
        .on_click(move |event, window, app| {
            if accepts_activation_click(event) {
                click_action(window, app);
                app.stop_propagation();
            }
        })
        .on_key_down(move |event: &KeyDownEvent, window, app| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") && !event.is_held {
                action(window, app);
                app.stop_propagation();
            }
        })
        .child(glyph)
}

fn accepts_activation_click(event: &ClickEvent) -> bool {
    matches!(event, ClickEvent::Mouse(_))
}

fn is_double_click(click_count: usize) -> bool {
    click_count >= 2
}

#[cfg(test)]
mod tests {
    use super::is_double_click;

    #[test]
    fn only_repeated_clicks_toggle_fullscreen() {
        assert!(!is_double_click(0));
        assert!(!is_double_click(1));
        assert!(is_double_click(2));
        assert!(is_double_click(3));
    }

    #[test]
    fn only_mouse_clicks_activate_bar_buttons() {
        use super::accepts_activation_click;
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
