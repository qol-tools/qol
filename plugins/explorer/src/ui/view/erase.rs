use gpui::{
    div, px, rgb, AnyElement, Context, InteractiveElement, IntoElement, Keystroke, ParentElement,
    Styled,
};
use qol_gpui::kit::kit;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{TextStyle, SPACE_GUTTER, SPACE_PAD, SPACE_TIGHT};

use super::clash::buttons;
use super::{ExplorerView, BAND};
use crate::ui::browse::erase_lines;

pub(super) fn render(view: &ExplorerView, cx: &mut Context<ExplorerView>) -> AnyElement {
    let kit = kit();
    let pane = kit.grounds.pane;
    let lines = view.browser.erasing().map(erase_lines).unwrap_or_default();
    div()
        .absolute()
        .top(px(BAND))
        .left_0()
        .right_0()
        .bottom_0()
        .occlude()
        .flex()
        .flex_col()
        .bg(rgb(pane.bg))
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(SPACE_TIGHT))
                .px(px(SPACE_GUTTER))
                .py(px(SPACE_PAD))
                .children(lines.into_iter().map(|line| {
                    div()
                        .text(TextStyle::Name)
                        .text_color(rgb(pane.ink))
                        .child(line)
                })),
        )
        .child(div().flex_1())
        .child(buttons(
            kit,
            "Delete permanently",
            false,
            ExplorerView::cancel_erase,
            ExplorerView::confirm_erase,
            cx,
        ))
        .into_any_element()
}

impl ExplorerView {
    pub(super) fn cancel_erase(&mut self, cx: &mut Context<Self>) {
        self.browser.erase_cancel();
        cx.notify();
    }

    pub(super) fn confirm_erase(&mut self, cx: &mut Context<Self>) {
        self.browser.erase_confirm();
        cx.notify();
    }

    pub(super) fn erase_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        match keystroke.key.as_str() {
            "escape" => self.cancel_erase(cx),
            "enter" => self.confirm_erase(cx),
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }
}
