use gpui::prelude::*;
use gpui::{div, px, rgb, App, CursorStyle, FontWeight, SharedString, Window};

use crate::icon_button::IconButton;
use crate::surface::PanelDragArea;
use crate::theme::{SystemPalette, DARK_REFERENCE};

const BAR_HEIGHT: f32 = 34.0;
const BAR_PADDING_X: f32 = 12.0;
const TRAILING_GAP: f32 = 9.0;
const TITLE_SIZE: f32 = 11.0;

const fn chrome_defaults() -> (u32, u32, u32) {
    let system = SystemPalette::from_reference(DARK_REFERENCE);
    (
        system.surface_canvas,
        system.text_secondary,
        system.text_muted,
    )
}

fn divider_default(system: &SystemPalette) -> u32 {
    mix_rgb(system.surface_elevated, system.border_subtle, 0.5)
}

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
    children: Vec<gpui::AnyElement>,
}

impl WindowBar {
    pub fn new(title: impl Into<SharedString>) -> Self {
        let system = SystemPalette::from_reference(DARK_REFERENCE);
        let (background, title_color, button_text) = chrome_defaults();
        let border = divider_default(&system);
        Self {
            title: title.into(),
            background,
            border,
            title_color,
            button_text,
            button_hover: 0xffffff0f,
            button_focus: system.accent,
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
            trailing = trailing.child(
                IconButton::new("window-bar-collapse", "\u{2581}")
                    .style(self.button_text, self.button_hover, self.button_focus)
                    .on_activate(action),
            );
        }
        if let Some(action) = self.hide {
            trailing = trailing.child(
                IconButton::new("window-bar-hide", "\u{00D7}")
                    .style(self.button_text, self.button_hover, self.button_focus)
                    .on_activate(action),
            );
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

fn is_double_click(click_count: usize) -> bool {
    click_count >= 2
}

fn mix_rgb(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xff) as f32;
        let y = ((b >> shift) & 0xff) as f32;
        ((x + (y - x) * t).round() as u32 & 0xff) << shift
    };
    channel(16) | channel(8) | channel(0)
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
}
