use gpui::{
    div, rgb, AnyElement, App, Context, InteractiveElement, IntoElement, Keystroke, MouseDownEvent,
    ParentElement, RenderOnce, Styled, Window,
};
use qol_gpui::text::laid_out_width;
use qol_gpui::text_edit::{visible_char_count, CaretStyle, TextField, TextFieldElement};
use qol_gpui::theme::{Ground, TextStyle, SPACE_CELL, SPACE_GUTTER};

use super::list::NAME_MAX_WIDTH;
use super::{ExplorerView, ICON};

#[derive(IntoElement)]
pub(super) struct RenameLabel {
    pub field: TextField,
    pub available: f32,
    pub style: TextStyle,
    pub ink: u32,
    pub mark: u32,
    pub on_mark: u32,
}

impl RenameLabel {
    pub(super) fn on_ground(
        field: TextField,
        available: f32,
        style: TextStyle,
        ground: Ground,
    ) -> Self {
        Self {
            field,
            available,
            style,
            ink: ground.ink,
            mark: ground.mark,
            on_mark: ground.on_mark,
        }
    }
}

impl RenderOnce for RenameLabel {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let advance = laid_out_width(window, "0", self.style);
        let visible = visible_char_count(self.available, advance);
        TextFieldElement::new(&self.field, visible, advance)
            .with_marks()
            .measure(|shown| laid_out_width(window, shown, self.style))
            .selection(rgb(self.mark).into(), Some(rgb(self.on_mark).into()))
            .caret(CaretStyle {
                color: rgb(self.ink).into(),
                width: 2.0,
                height: 16.0,
                top: 1.0,
                radius: 1.0,
            })
            .render()
    }
}

pub(super) fn row_name_room(view: &ExplorerView, columns: bool) -> f32 {
    let beside = if columns {
        super::list::COLUMNS_WIDTH
    } else {
        0.0
    };
    (view.list_width - 2.0 * SPACE_GUTTER - ICON - SPACE_CELL - beside).clamp(0.0, NAME_MAX_WIDTH)
}

impl ExplorerView {
    pub(super) fn rename_label(
        &self,
        available: f32,
        style: TextStyle,
        ground: Ground,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let field = self.browser.rename_field()?.field().clone();
        let label = RenameLabel::on_ground(field, available, style, ground);
        let outside = cx.listener(|view, _: &MouseDownEvent, _window, cx| {
            view.commit_rename(cx);
        });
        Some(
            div()
                .min_w_0()
                .flex_1()
                .on_mouse_down_out(outside)
                .child(label)
                .into_any_element(),
        )
    }

    pub(super) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        if let Ok(enter) = Keystroke::parse("enter") {
            self.browser.rename_key(&enter, cx);
        }
        cx.notify();
    }

    pub(super) fn rename_pressed(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        self.browser.rename_key(keystroke, cx);
        cx.stop_propagation();
        cx.notify();
    }
}
