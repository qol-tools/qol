use gpui::*;

use super::layout::HEADER_HEIGHT;
use super::search::Scored;

const BG: u32 = 0x1e1e2e;
const BG_SELECTED: u32 = 0x45475a;
const TEXT: u32 = 0xcdd6f4;
const TEXT_DIM: u32 = 0xa6adc8;
const TEXT_MUTED: u32 = 0x6c7086;
const HIGHLIGHT: u32 = 0xf9e2af;
const BORDER: u32 = 0x45475a;

pub fn search_bar(
    mode_label: &'static str,
    fuzziness_label: &'static str,
    query: &str,
    cursor: usize,
    selection: Option<(usize, usize)>,
) -> Div {
    div()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .px_4()
        .gap_2()
        .bg(rgb(BG))
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(
            div()
                .h(px(20.))
                .px_2()
                .flex()
                .items_center()
                .bg(rgb(BG_SELECTED))
                .text_color(rgb(TEXT_DIM))
                .text_size(px(12.))
                .child(mode_label),
        )
        .child(
            div()
                .h(px(20.))
                .px_2()
                .flex()
                .items_center()
                .bg(rgb(BG_SELECTED))
                .text_color(rgb(TEXT_DIM))
                .text_size(px(12.))
                .child(fuzziness_label),
        )
        .child(div().text_color(rgb(TEXT_MUTED)).text_size(px(16.)).child(">"))
        .child(
            div()
                .flex_1()
                .text_size(px(16.))
                .flex()
                .items_center()
                .children(search_bar_content(query, cursor, selection)),
        )
}

fn search_bar_content(query: &str, cursor: usize, selection: Option<(usize, usize)>) -> Vec<AnyElement> {
    if query.is_empty() {
        return vec![div()
            .text_color(rgb(TEXT_MUTED))
            .child("Type to search...")
            .into_any_element()];
    }

    let chars: Vec<char> = query.chars().collect();
    let mut out = Vec::with_capacity(chars.len() + 1);
    let (sel_start, sel_end) = selection.unwrap_or((usize::MAX, usize::MAX));
    let show_caret = selection.is_none();

    for (i, ch) in chars.iter().enumerate() {
        if show_caret && i == cursor {
            out.push(
                div()
                    .text_color(rgb(HIGHLIGHT))
                    .child("|")
                    .into_any_element(),
            );
        }
        let selected = i >= sel_start && i < sel_end;
        let mut glyph = div().child(ch.to_string());
        if selected {
            glyph = glyph.bg(rgb(BG_SELECTED)).text_color(rgb(TEXT));
        } else {
            glyph = glyph.text_color(rgb(TEXT));
        }
        out.push(glyph.into_any_element());
    }

    if show_caret && cursor >= chars.len() {
        out.push(
            div()
                .text_color(rgb(HIGHLIGHT))
                .child("|")
                .into_any_element(),
        );
    }

    out
}

pub fn result_row(scored: &Scored<'_>, selected: bool, row_height: f32) -> Div {
    let positions = &scored.m.positions;
    let base_color = if selected { rgb(TEXT) } else { rgb(TEXT_DIM) };
    let bg = if selected { rgb(BG_SELECTED) } else { rgb(BG) };

    let spans: Vec<AnyElement> = scored
        .item
        .name()
        .char_indices()
        .map(|(i, ch)| {
            let color = if positions.contains(&i) { rgb(HIGHLIGHT) } else { base_color };
            div()
                .text_color(color)
                .text_size(px(14.))
                .child(ch.to_string())
                .into_any_element()
        })
        .collect();

    div()
        .h(px(row_height))
        .w_full()
        .flex()
        .items_center()
        .px_4()
        .bg(bg)
        .children(spans)
}

pub fn bg_color() -> gpui::Rgba {
    rgb(BG)
}
