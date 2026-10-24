use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, rgb, rgba, AnyElement, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};
use qol_gpui::kit::{kit, Kit};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{
    clear, TextStyle, HEIGHT_INLINE, LINE, SPACE_CELL, SPACE_GUTTER, SPACE_PAD, SPACE_STACK,
};

use super::{ExplorerView, BAND};
use crate::ui::browse::{MoveRecord, MoveState};

const ROW_HEIGHT: f32 = 60.0;
const SIZE_WIDTH: f32 = 100.0;

pub(super) fn render(view: &ExplorerView) -> AnyElement {
    let kit = kit();
    let pane = kit.grounds.pane;
    let records = view.browser.moves();
    let body = if records.is_empty() {
        kit.empty(
            "No moves yet",
            Some("Copies and moves you start today are listed here".into()),
        )
        .into_any_element()
    } else {
        let source = ScrollSource::Handle {
            handle: view.moves_scroll.handle().clone(),
            children: records.len(),
        };
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                div()
                    .id("explorer-moves-list")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(view.moves_scroll.handle())
                    .children(
                        records
                            .iter()
                            .enumerate()
                            .map(|(index, record)| row(index, record, kit)),
                    ),
            )
            .child(kit.scroll_cue(source, pane))
            .into_any_element()
    };
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
                .child("Moves today"),
        )
        .child(body)
        .child(
            div()
                .flex_none()
                .px(px(SPACE_GUTTER))
                .py(px(SPACE_PAD))
                .text(TextStyle::Hint)
                .text_color(rgb(pane.faint))
                .child("Nothing waits for you: each question came before its move started."),
        )
        .into_any_element()
}

fn tone(state: &MoveState, kit: Kit) -> u32 {
    match state {
        MoveState::Running => kit.palette.accent,
        MoveState::Done => kit.palette.success,
        MoveState::NotStarted => kit.palette.text_muted,
        MoveState::Failed => kit.palette.danger,
    }
}

fn row(index: usize, record: &MoveRecord, kit: Kit) -> AnyElement {
    let pane = kit.grounds.pane;
    let dot = tone(&record.state, kit);
    div()
        .id(("explorer-move", index))
        .flex_none()
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_CELL))
        .px(px(SPACE_GUTTER))
        .border_b(px(LINE))
        .border_color(rgba(kit.washes.separator.packed()))
        .child(kit.status_dot(dot, clear(dot)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(SPACE_STACK))
                .child(
                    div()
                        .text(TextStyle::ListName)
                        .text_color(rgb(pane.ink))
                        .child(record.title.clone()),
                )
                .child(
                    div()
                        .text(TextStyle::Detail)
                        .text_color(rgb(pane.soft))
                        .child(record.detail.clone()),
                ),
        )
        .when_some(record.bytes, |row, bytes| {
            row.child(
                div()
                    .flex_none()
                    .w(px(SIZE_WIDTH))
                    .flex()
                    .justify_end()
                    .text(TextStyle::Detail)
                    .text_color(rgb(pane.soft))
                    .child(qol_gpui::format_bytes(bytes)),
            )
        })
        .into_any_element()
}

pub(super) fn progress_label(done: u64, total: u64) -> String {
    let done = qol_gpui::format_bytes(done);
    let total = qol_gpui::format_bytes(total);
    match (done.rsplit_once(' '), total.rsplit_once(' ')) {
        (Some((number, unit)), Some((_, total_unit))) if unit == total_unit => {
            format!("{number} of {total}")
        }
        _ => format!("{done} of {total}"),
    }
}

#[cfg(test)]
mod tests {
    use super::progress_label;

    #[test]
    fn progress_shares_one_unit_when_both_sides_use_it() {
        let cases = [
            (3_100_000_000, 8_500_000_000, "3.1 of 8.5 GB"),
            (900_000_000, 8_500_000_000, "900 MB of 8.5 GB"),
            (0, 700, "0 of 700 B"),
        ];
        for (done, total, expected) in cases {
            assert_eq!(progress_label(done, total), expected, "{done} {total}");
        }
    }
}
