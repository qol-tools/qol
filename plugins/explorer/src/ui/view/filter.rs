use gpui::{div, px, rgb, rgba, AnyElement, IntoElement, ParentElement, Styled, Window};
use qol_gpui::kit::kit;
use qol_gpui::text::{laid_out_width, TextStyled};
use qol_gpui::text_edit::{visible_char_count, CaretStyle, TextField, TextFieldElement};
use qol_gpui::theme::{TextStyle, HEIGHT_CONTROL, RADIUS_CONTROL, SPACE_CELL};

pub(super) const WIDTH: f32 = 200.0;
const CARET_WIDTH: f32 = 2.0;
const STYLE: TextStyle = TextStyle::ListName;

pub(super) fn render(field: &TextField, window: &mut Window) -> AnyElement {
    let kit = kit();
    let pane = kit.grounds.pane;
    let advance = laid_out_width(window, "0", STYLE);
    let visible = visible_char_count(WIDTH - 2.0 * SPACE_CELL - CARET_WIDTH, advance);
    let text = TextFieldElement::new(field, visible, advance)
        .with_marks()
        .measure(|shown| laid_out_width(window, shown, STYLE))
        .selection(rgb(kit.grounds.band.bg).into(), Some(rgb(pane.ink).into()))
        .caret(CaretStyle {
            color: rgb(pane.ink).into(),
            width: CARET_WIDTH,
            height: 16.0,
            top: 1.0,
            radius: 1.0,
        })
        .placeholder("Filter names", rgb(pane.faint).into())
        .render();
    let shell = div()
        .flex_none()
        .w(px(WIDTH))
        .h(px(HEIGHT_CONTROL))
        .px(px(SPACE_CELL))
        .flex()
        .items_center()
        .rounded(px(RADIUS_CONTROL))
        .bg(rgba(pane.well.packed()))
        .text(STYLE)
        .text_color(rgb(pane.ink))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(text),
        );
    kit.ringed(shell, pane, RADIUS_CONTROL).into_any_element()
}
