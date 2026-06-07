use std::ops::Range;

use gpui::*;

use super::layout::HEADER_HEIGHT;
use crate::discovery::search::{MatchKind, Scored};

const BG: u32 = 0x1e1e2e;
const BG_SELECTED: u32 = 0x5a607d;
const BG_TRAIL_HOT: u32 = 0x2b3043;
const BG_TRAIL: u32 = 0x262a3b;
const BG_NEAR: u32 = 0x212435;
const BG_EDGE: u32 = 0x1f2132;
const BG_BADGE: u32 = 0x313244;
const TEXT: u32 = 0xcdd6f4;
const TEXT_SELECTED: u32 = 0xf5f7ff;
const TEXT_DIM: u32 = 0x8f96b3;
const TEXT_MUTED: u32 = 0x6d738c;
const TEXT_FAINT: u32 = 0x585d72;
const HIGHLIGHT: u32 = 0xf9e2af;
const HIGHLIGHT_WARM: u32 = 0xf7dc9f;
const HIGHLIGHT_HOT: u32 = 0xfde5b8;
const HIGHLIGHT_COOL: u32 = 0xe6c983;
const OVERFLOW: u32 = 0x6f88b8;
const EDGE_PULSE: u32 = 0xaa7383;
const BORDER: u32 = 0x45475a;
const MOMENTUM_UP_1: u32 = 0x3a4761;
const MOMENTUM_UP_2: u32 = 0x35527a;
const MOMENTUM_UP_3: u32 = 0x2f5e8c;
const MOMENTUM_DOWN_1: u32 = 0x4c4650;
const MOMENTUM_DOWN_2: u32 = 0x5e4851;
const MOMENTUM_DOWN_3: u32 = 0x714b53;
const COMPASS_UP_LOW: u32 = 0x5b6f9a;
const COMPASS_UP_MID: u32 = 0x7390ca;
const COMPASS_UP_HIGH: u32 = 0x8fb1ff;
const COMPASS_DOWN_LOW: u32 = 0x8a6672;
const COMPASS_DOWN_MID: u32 = 0xb88290;
const COMPASS_DOWN_HIGH: u32 = 0xd69aa8;
const SEMANTIC_PREFIX: u32 = 0x6a8bc6;
const SEMANTIC_CONTAINS: u32 = 0x7c709e;
const SEMANTIC_FUZZY: u32 = 0x5f6276;
const SEMANTIC_FREQ: u32 = 0x7d5f7f;
const BOOST_BG: u32 = 0x4a6741;

#[allow(clippy::too_many_arguments)]
pub fn search_bar(
    mode_label: &'static str,
    fuzziness_label: &'static str,
    query: &str,
    cursor: usize,
    selection: Option<(usize, usize)>,
    selected: usize,
    result_count: usize,
    scroll_offset: usize,
    visible: usize,
    momentum_signed: i8,
    hidden_above: usize,
    hidden_below: usize,
) -> Div {
    let momentum_bg = momentum_badge_bg(momentum_signed);
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
                .bg(momentum_bg)
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
                .bg(momentum_bg)
                .text_color(rgb(TEXT_DIM))
                .text_size(px(12.))
                .child(fuzziness_label),
        )
        .child(
            div()
                .text_color(rgb(TEXT_MUTED))
                .text_size(px(16.))
                .child(">"),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .rounded_full()
                .font_family(SharedString::from("Menlo"))
                .text_size(px(14.))
                .flex()
                .items_center()
                .child(search_bar_content(query, cursor, selection)),
        )
        .child(compass_widget(hidden_above, hidden_below))
        .child(browse_status(
            selected,
            result_count,
            scroll_offset,
            visible,
        ))
}

const SEARCH_VISIBLE_CHARS: usize = 25;

fn search_bar_content(query: &str, cursor: usize, selection: Option<(usize, usize)>) -> AnyElement {
    if query.is_empty() {
        return div()
            .text_color(rgb(TEXT_MUTED))
            .child("Type to search...")
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
                    color: Some(rgb(TEXT).into()),
                    background_color: Some(rgb(BG_SELECTED).into()),
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
            HighlightStyle::color(rgb(HIGHLIGHT).into()),
        ));
    }

    let styled = StyledText::new(SharedString::from(display)).with_highlights(highlights);
    div().text_color(rgb(TEXT)).child(styled).into_any_element()
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

#[derive(Clone, Copy)]
pub struct RowWindowCue {
    pub selected: bool,
    pub previous_selected: bool,
    pub trail_depth: u8,
    pub distance_from_selected: usize,
    pub hidden_above: usize,
    pub hidden_below: usize,
    pub edge_hit_top: bool,
    pub edge_hit_bottom: bool,
    pub confidence_pct: u8,
    pub cluster_break: bool,
}

pub fn result_row(scored: &Scored, name: &str, cues: RowWindowCue, row_height: f32) -> Div {
    let base_color = row_text_color(cues.selected, cues.previous_selected, cues.trail_depth);
    let bg = row_bg_color(
        cues.selected,
        cues.previous_selected,
        cues.trail_depth,
        cues.distance_from_selected,
    );

    if !cues.selected {
        let mut row = div()
            .h(px(row_height))
            .w_full()
            .flex()
            .items_center()
            .px_4()
            .bg(bg)
            .text_color(base_color)
            .text_size(px(14.))
            .child(name.to_owned());
        if scored.manual_boost > 0 {
            row = row.child(boost_badge(scored.manual_boost, false));
        }
        return row;
    }

    let positions = &scored.m.positions;
    let highlights = if !positions.is_empty() {
        char_highlights(name, positions)
    } else {
        vec![]
    };
    let styled_name =
        StyledText::new(SharedString::from(name.to_owned())).with_highlights(highlights);

    let mut row = div()
        .h(px(row_height))
        .w_full()
        .flex()
        .items_center()
        .px_4()
        .bg(bg);

    if cues.cluster_break {
        row = row.child(cluster_badge());
    }

    if cues.edge_hit_top {
        row = row.child(edge_badge("^"));
    }
    if !cues.edge_hit_top && cues.hidden_above > 0 {
        row = row.child(overflow_badge("^", cues.hidden_above));
    }

    row = row.child(
        div()
            .flex_1()
            .flex()
            .items_center()
            .text_color(base_color)
            .text_size(px(14.))
            .child(styled_name),
    );

    if scored.manual_boost > 0 {
        row = row.child(boost_badge(scored.manual_boost, true));
    }
    row = row.child(semantic_badge(
        scored.match_kind,
        scored.frecency_bonus > 0,
        true,
    ));
    row = row.child(confidence_bar(cues.confidence_pct, true));

    if cues.edge_hit_bottom {
        row = row.child(edge_badge("v"));
    }
    if !cues.edge_hit_bottom && cues.hidden_below > 0 {
        row = row.child(overflow_badge("v", cues.hidden_below));
    }

    row
}

fn browse_status(
    selected: usize,
    result_count: usize,
    scroll_offset: usize,
    visible: usize,
) -> Div {
    let (selection_label, range_label) = if result_count == 0 {
        ("0/0".to_string(), "0-0".to_string())
    } else {
        let selected_index = selected.min(result_count.saturating_sub(1)) + 1;
        let start = (scroll_offset + 1).min(result_count);
        let end = (scroll_offset + visible).min(result_count);
        (
            format!("{selected_index}/{result_count}"),
            format!("{start}-{end}"),
        )
    };

    div()
        .flex()
        .items_center()
        .gap_2()
        .child(status_badge(selection_label))
        .child(status_badge(range_label))
}

fn status_badge(label: String) -> Div {
    div()
        .h(px(20.))
        .px_2()
        .flex()
        .items_center()
        .bg(rgb(BG_BADGE))
        .text_color(rgb(TEXT_DIM))
        .text_size(px(12.))
        .child(label)
}

fn overflow_badge(direction: &str, count: usize) -> Div {
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(rgb(BG_BADGE))
        .text_color(rgb(OVERFLOW))
        .text_size(px(11.))
        .child(format!("{direction} {count}"))
}

fn edge_badge(direction: &str) -> Div {
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(rgb(BG_BADGE))
        .text_color(rgb(EDGE_PULSE))
        .text_size(px(11.))
        .child(format!("{direction} edge"))
}

fn confidence_bar(confidence_pct: u8, selected: bool) -> Div {
    let slots = 6usize;
    let filled = (confidence_pct as usize * slots).div_ceil(100);
    let filled = filled.min(slots);
    let label = format!("[{}{}]", "=".repeat(filled), ".".repeat(slots - filled));
    div()
        .h(px(18.))
        .w(px(52.))
        .flex()
        .items_center()
        .justify_end()
        .text_color(if selected {
            rgb(TEXT_DIM)
        } else {
            rgb(TEXT_FAINT)
        })
        .text_size(px(10.))
        .child(label)
}

fn cluster_badge() -> Div {
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(rgb(BG_BADGE))
        .text_color(rgb(TEXT_DIM))
        .text_size(px(10.))
        .child("gap")
}

fn boost_badge(boost: i32, selected: bool) -> Div {
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(if selected { rgb(BOOST_BG) } else { rgb(BG) })
        .text_color(if selected {
            rgb(TEXT_SELECTED)
        } else {
            rgb(TEXT_FAINT)
        })
        .text_size(px(10.))
        .child(format!("+{boost}"))
}

fn semantic_badge(kind: MatchKind, freq_bonus: bool, selected: bool) -> Div {
    let base = match kind {
        MatchKind::Prefix => "prefix",
        MatchKind::Contains => "contains",
        MatchKind::Fuzzy => "fuzzy",
    };
    let label = if freq_bonus {
        format!("{base}+freq")
    } else {
        base.to_string()
    };
    let bg = if selected {
        if freq_bonus {
            rgb(SEMANTIC_FREQ)
        } else {
            match kind {
                MatchKind::Prefix => rgb(SEMANTIC_PREFIX),
                MatchKind::Contains => rgb(SEMANTIC_CONTAINS),
                MatchKind::Fuzzy => rgb(SEMANTIC_FUZZY),
            }
        }
    } else {
        rgb(BG)
    };
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(bg)
        .text_color(if selected {
            rgb(TEXT_SELECTED)
        } else {
            rgb(TEXT_FAINT)
        })
        .text_size(px(10.))
        .child(label)
}

fn momentum_badge_bg(momentum_signed: i8) -> gpui::Rgba {
    match momentum_signed {
        -3..=-1 => match momentum_signed.abs() {
            1 => rgb(MOMENTUM_UP_1),
            2 => rgb(MOMENTUM_UP_2),
            _ => rgb(MOMENTUM_UP_3),
        },
        1..=3 => match momentum_signed {
            1 => rgb(MOMENTUM_DOWN_1),
            2 => rgb(MOMENTUM_DOWN_2),
            _ => rgb(MOMENTUM_DOWN_3),
        },
        _ => rgb(BG_SELECTED),
    }
}

fn compass_widget(hidden_above: usize, hidden_below: usize) -> Div {
    div()
        .h(px(20.))
        .w(px(10.))
        .flex()
        .flex_col()
        .justify_between()
        .child(compass_strip(hidden_above, true))
        .child(compass_strip(hidden_below, false))
}

fn compass_strip(hidden_count: usize, is_up: bool) -> Div {
    let color = if hidden_count == 0 {
        rgb(BG_EDGE)
    } else if hidden_count <= 3 {
        if is_up {
            rgb(COMPASS_UP_LOW)
        } else {
            rgb(COMPASS_DOWN_LOW)
        }
    } else if hidden_count <= 7 {
        if is_up {
            rgb(COMPASS_UP_MID)
        } else {
            rgb(COMPASS_DOWN_MID)
        }
    } else if is_up {
        rgb(COMPASS_UP_HIGH)
    } else {
        rgb(COMPASS_DOWN_HIGH)
    };
    div().h(px(3.)).w_full().bg(color)
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
            let color: Hsla = match_heat_color(char_idx, positions).into();
            Some((byte_pos..byte_pos + byte_len, HighlightStyle::color(color)))
        })
        .collect()
}

fn match_heat_color(index: usize, positions: &[usize]) -> gpui::Rgba {
    let contiguous_left = index > 0 && positions.contains(&(index - 1));
    let contiguous_right = positions.contains(&(index + 1));
    if contiguous_left && contiguous_right {
        rgb(HIGHLIGHT_HOT)
    } else if contiguous_left || contiguous_right {
        rgb(HIGHLIGHT_WARM)
    } else {
        rgb(HIGHLIGHT_COOL)
    }
}

fn row_text_color(selected: bool, previous_selected: bool, trail_depth: u8) -> gpui::Rgba {
    if selected {
        rgb(TEXT_SELECTED)
    } else if trail_depth >= 2 {
        rgb(TEXT_DIM)
    } else if trail_depth == 1 || previous_selected {
        rgb(TEXT_MUTED)
    } else {
        rgb(TEXT_FAINT)
    }
}

fn row_bg_color(
    selected: bool,
    previous_selected: bool,
    trail_depth: u8,
    distance_from_selected: usize,
) -> gpui::Rgba {
    if selected {
        rgb(BG_SELECTED)
    } else if trail_depth >= 2 {
        rgb(BG_TRAIL_HOT)
    } else if trail_depth == 1 {
        rgb(BG_TRAIL)
    } else if previous_selected {
        rgb(BG_NEAR)
    } else if distance_from_selected <= 1 {
        rgb(BG_EDGE)
    } else {
        rgb(BG)
    }
}

pub fn bg_color() -> gpui::Rgba {
    rgb(BG)
}
