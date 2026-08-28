use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::hint_bar::{estimated_chip_width, fit_hints, BarItem, HintDescriptor};
use qol_gpui::theme::{launcher_runtime, LauncherPalette, TEXT_BODY, TEXT_NANO};

use super::layout::{HEADER_HEIGHT, WINDOW_WIDTH};
use crate::discovery::search::{ResultSource, Scored, SearchMode};
use crate::flow::{FlowEntry, FlowRow};

fn current_palette() -> LauncherPalette {
    launcher_runtime()
}

pub fn palette() -> LauncherPalette {
    current_palette()
}

pub fn search_bar(
    query: &str,
    launch_error: Option<&str>,
    cursor: usize,
    selection: Option<(usize, usize)>,
    selected: usize,
    result_count: usize,
    placeholder: &str,
) -> Div {
    let kit = qol_gpui::kit::kit();
    let counter = if result_count == 0 {
        format!("0 / {result_count}")
    } else {
        format!("{} / {result_count}", selected.min(result_count - 1) + 1)
    };
    div()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .px(px(qol_gpui::theme::SPACE_PAD))
        .gap(px(10.0))
        .bg(rgb(current_palette().bg))
        .border_b(px(1.0))
        .border_color(rgba(if result_count == 0 {
            0
        } else {
            kit.washes.hairline.packed()
        }))
        .child(
            div()
                .flex_none()
                .text_color(rgb(kit.palette.accent_ink))
                .text_size(px(TEXT_BODY))
                .font_weight(FontWeight::BOLD)
                .child("\u{203A}"),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .flex()
                .flex_col()
                .justify_center()
                .child(
                    div()
                        .h(px(18.))
                        .overflow_hidden()
                        .text_size(px(TEXT_BODY))
                        .flex()
                        .items_center()
                        .child(search_bar_content(query, cursor, selection, placeholder)),
                )
                .when_some(launch_error, |field, error| {
                    field.child(
                        div()
                            .h(px(12.))
                            .overflow_hidden()
                            .text_color(rgb(current_palette().highlight_warm))
                            .text_size(px(TEXT_NANO))
                            .child(error.to_owned()),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .text_color(rgb(kit.palette.text_muted))
                .text_size(px(TEXT_NANO))
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .child(counter),
        )
}

const SEARCH_VISIBLE_CHARS: usize = 25;

fn search_bar_content(
    query: &str,
    cursor: usize,
    selection: Option<(usize, usize)>,
    placeholder: &str,
) -> AnyElement {
    if query.is_empty() {
        return div()
            .text_color(rgb(current_palette().text_muted))
            .child(placeholder.to_owned())
            .into_any_element();
    }

    let char_count = query.chars().count();

    let view_start = if char_count <= SEARCH_VISIBLE_CHARS {
        0
    } else {
        cursor
            .saturating_sub(SEARCH_VISIBLE_CHARS.saturating_sub(2))
            .min(char_count.saturating_sub(SEARCH_VISIBLE_CHARS))
    };
    let view_end = (view_start + SEARCH_VISIBLE_CHARS).min(char_count);

    let start_byte = char_to_byte(query, view_start);
    let end_byte = char_to_byte(query, view_end);
    let visible = &query[start_byte..end_byte];

    let mut display = visible.to_owned();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();

    if let Some((sel_start, sel_end)) = selection {
        let adj_start = sel_start
            .saturating_sub(view_start)
            .min(view_end - view_start);
        let adj_end = sel_end
            .saturating_sub(view_start)
            .min(view_end - view_start);
        let start = char_to_byte(visible, adj_start);
        let end = char_to_byte(visible, adj_end);
        if start < end {
            highlights.push((
                start..end,
                HighlightStyle {
                    color: Some(rgb(current_palette().text).into()),
                    background_color: Some(rgb(current_palette().bg_selected).into()),
                    ..HighlightStyle::default()
                },
            ));
        }
    }

    if selection.is_none() {
        let adj_cursor = cursor
            .saturating_sub(view_start)
            .min(display.chars().count());
        let caret_byte = char_to_byte(&display, adj_cursor);
        display.insert(caret_byte, '|');
        highlights.push((
            caret_byte..caret_byte + 1,
            HighlightStyle::color(rgb(current_palette().highlight).into()),
        ));
    }

    let styled = StyledText::new(SharedString::from(display)).with_highlights(highlights);
    div()
        .text_color(rgb(current_palette().text))
        .child(styled)
        .into_any_element()
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

pub fn result_row(scored: &Scored, name: &str, selected: bool, row_height: f32) -> Div {
    let kit = qol_gpui::kit::kit();
    let positions = &scored.m.positions;
    let highlights = if selected && !positions.is_empty() {
        char_highlights(name, positions)
    } else {
        vec![]
    };
    let styled_name =
        StyledText::new(SharedString::from(name.to_owned())).with_highlights(highlights);
    let row = div()
        .flex_none()
        .h(px(row_height))
        .mx(px(8.0))
        .px(px(qol_gpui::theme::SPACE_PAD))
        .flex()
        .items_center()
        .gap(px(12.0))
        .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
        .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
        .child(kit.letter_tile(name))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_color(rgb(if selected {
                    current_palette().text
                } else {
                    current_palette().text_muted
                }))
                .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                .child(styled_name),
        )
        .child(
            div()
                .flex_none()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .text_color(rgb(kit.palette.text_secondary))
                .text_size(px(qol_gpui::theme::TEXT_MICRO))
                .child(kind_label(scored.source)),
        );
    kit.row_selected(row, selected)
}

pub fn flow_row(row: &FlowRow, selected: bool, row_height: f32) -> Div {
    let kit = qol_gpui::kit::kit();
    let text = div()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .flex()
        .flex_col()
        .justify_center()
        .child(
            div()
                .truncate()
                .text_color(rgb(if selected {
                    current_palette().text
                } else {
                    current_palette().text_muted
                }))
                .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                .child(row.title.clone()),
        )
        .when_some(row.subtitle.as_deref(), |text, subtitle| {
            text.child(
                div()
                    .truncate()
                    .text_color(rgb(current_palette().text_muted))
                    .text_size(px(qol_gpui::theme::TEXT_NANO))
                    .child(subtitle.to_owned()),
            )
        });
    let element = div()
        .flex_none()
        .h(px(row_height))
        .mx(px(8.0))
        .px(px(qol_gpui::theme::SPACE_PAD))
        .flex()
        .items_center()
        .gap(px(12.0))
        .rounded(px(qol_gpui::theme::RADIUS_CONTROL))
        .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
        .child(kit.letter_tile(&row.title))
        .child(text);
    kit.row_selected(element, selected)
}

pub fn hint_bar_flow(entry: &FlowEntry) -> Div {
    let kit = qol_gpui::kit::kit();
    let enter_label = entry
        .row_actions
        .first()
        .and_then(|action| action.label.as_deref())
        .unwrap_or("copy");
    kit.hint_bar()
        .child(kit.hint("\u{23CE}", enter_label.to_owned()))
        .child(kit.hint("\u{2191}\u{2193}", "move"))
        .child(kit.hint("esc", "back"))
        .child(kit.chip(entry.title.clone(), kit.palette.accent))
        .child(div().flex_1())
}

fn kind_label(source: ResultSource) -> &'static str {
    match source {
        ResultSource::App => "app",
        ResultSource::File => "dir",
        ResultSource::Flow => "flow",
    }
}

fn char_highlights(name: &str, positions: &[usize]) -> Vec<(Range<usize>, HighlightStyle)> {
    let byte_map: Vec<(usize, usize)> = name
        .char_indices()
        .map(|(byte_pos, ch)| (byte_pos, ch.len_utf8()))
        .collect();

    positions
        .iter()
        .filter_map(|&char_idx| {
            let &(byte_pos, byte_len) = byte_map.get(char_idx)?;
            Some((
                byte_pos..byte_pos + byte_len,
                HighlightStyle {
                    color: Some(rgb(current_palette().highlight).into()),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
            ))
        })
        .collect()
}

pub fn hint_bar(mode: SearchMode) -> Div {
    let kit = qol_gpui::kit::kit();
    let items = [
        BarItem::Hint(HintDescriptor::new("\u{23CE}", "open", 2)),
        BarItem::Hint(HintDescriptor::new("\u{2191}\u{2193}", "move", 2)),
        BarItem::Hint(HintDescriptor::new("\u{21E5}", "mode", 1)),
        BarItem::FixedWidth(estimated_chip_width(mode.label())),
        BarItem::Hint(HintDescriptor::new("type", "search", 0)),
        BarItem::Spacer,
        BarItem::Hint(HintDescriptor::pinned("esc", "dismiss")),
    ];
    let mut bar = kit.hint_bar();
    for item in fit_hints(WINDOW_WIDTH, &items) {
        bar = match item {
            BarItem::Hint(spec) => bar.child(kit.hint(spec.key, spec.label)),
            BarItem::FixedWidth(_) => bar.child(kit.chip(mode.label(), kit.palette.accent)),
            BarItem::Spacer => bar.child(div().flex_1()),
        };
    }
    bar
}

pub fn bg_color() -> gpui::Rgba {
    rgb(current_palette().bg)
}
