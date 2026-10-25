use gpui::{
    div, img, px, rgb, rgba, AnyElement, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    StatefulInteractiveElement, Styled, StyledImage,
};
use qol_gpui::kit::kit;
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{
    TextStyle, HEIGHT_INLINE, LINE, SPACE_CELL, SPACE_GUTTER, SPACE_PAD, SPACE_STACK,
};

use super::{ExplorerView, BAND};
use crate::props::Props;

const LABEL_WIDTH: f32 = 112.0;
const PREVIEW_WIDTH: f32 = 360.0;
const PREVIEW_HEIGHT: f32 = 240.0;

pub(super) fn render(view: &ExplorerView) -> AnyElement {
    let kit = kit();
    let pane = kit.grounds.pane;
    let Some(props) = view.browser.props() else {
        return div().into_any_element();
    };
    let facts = props.rows();
    let rows = facts.len();
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
                .h(px(HEIGHT_INLINE))
                .flex()
                .items_center()
                .px(px(SPACE_GUTTER))
                .border_b(px(LINE))
                .border_color(rgba(kit.washes.hairline.packed()))
                .text(TextStyle::Label)
                .text_color(rgb(pane.faint))
                .child(heading(props)),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(
                    div()
                        .id("explorer-props-body")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&view.props_scroll)
                        .flex()
                        .flex_col()
                        .gap(px(SPACE_PAD))
                        .px(px(SPACE_GUTTER))
                        .py(px(SPACE_PAD))
                        .children(preview(props))
                        .child(div().flex().flex_col().gap(px(SPACE_STACK)).children(
                            facts.into_iter().map(|fact| {
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(SPACE_CELL))
                                    .child(
                                        div()
                                            .flex_none()
                                            .w(px(LABEL_WIDTH))
                                            .text(TextStyle::Detail)
                                            .text_color(rgb(pane.faint))
                                            .child(fact.label),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .text(TextStyle::ListName)
                                            .text_color(rgb(pane.ink))
                                            .child(fact.value),
                                    )
                            }),
                        )),
                )
                .child(kit.scroll_cue(
                    ScrollSource::Handle {
                        handle: view.props_scroll.clone(),
                        children: rows + usize::from(props.preview().is_some()),
                    },
                    pane,
                )),
        )
        .into_any_element()
}

fn heading(props: &Props) -> String {
    if props.counting() {
        format!("Properties of {}, counting size", props.title())
    } else {
        format!("Properties of {}", props.title())
    }
}

fn preview(props: &Props) -> Option<AnyElement> {
    let path = props.preview()?;
    Some(
        div()
            .flex_none()
            .w(px(PREVIEW_WIDTH))
            .h(px(PREVIEW_HEIGHT))
            .child(
                img(path.to_path_buf())
                    .w_full()
                    .h_full()
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element(),
    )
}
