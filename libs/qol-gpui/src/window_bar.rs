use gpui::prelude::*;
use gpui::{div, px, rgb, AnyElement, App, CursorStyle, FontWeight, SharedString, Window};

use crate::surface::PanelDragArea;

const BAR_HEIGHT: f32 = 34.0;
const BAR_PADDING_X: f32 = 12.0;
const TRAILING_GAP: f32 = 9.0;
const TITLE_SIZE: f32 = 11.0;
const DEFAULT_BACKGROUND: u32 = 0x14181f;
const DEFAULT_BORDER: u32 = 0x2f3644;
const DEFAULT_TITLE_COLOR: u32 = 0xd4dbea;

#[derive(IntoElement)]
pub struct WindowBar {
    title: SharedString,
    background: u32,
    border: u32,
    title_color: u32,
    children: Vec<AnyElement>,
}

impl WindowBar {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            background: DEFAULT_BACKGROUND,
            border: DEFAULT_BORDER,
            title_color: DEFAULT_TITLE_COLOR,
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

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}

impl RenderOnce for WindowBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let trailing = div()
            .flex()
            .items_center()
            .gap(px(TRAILING_GAP))
            .children(self.children);
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
