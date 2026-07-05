use std::ops::Range;
use std::sync::LazyLock;

use gpui::*;
use qol_gpui::theme::{launcher_dark, LauncherPalette};

use super::layout::HEADER_HEIGHT;
use crate::discovery::search::{MatchKind, Scored};

static CURRENT_PALETTE: LazyLock<LauncherPalette> = LazyLock::new(launcher_dark);

fn current_palette() -> &'static LauncherPalette {
    &CURRENT_PALETTE
}

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
        .bg(rgb(current_palette().bg))
        .border_b_1()
        .border_color(rgb(current_palette().border))
        .child(
            div()
                .h(px(20.))
                .px_2()
                .flex()
                .items_center()
                .bg(momentum_bg)
                .text_color(rgb(current_palette().text_dim))
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
                .text_color(rgb(current_palette().text_dim))
                .text_size(px(12.))
                .child(fuzziness_label),
        )
        .child(
            div()
                .text_color(rgb(current_palette().text_muted))
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
            .text_color(rgb(current_palette().text_muted))
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

#[derive(Clone, Copy)]
pub struct RowWindowCue {
    pub selected: bool,
    pub previous_selected: bool,
    pub trail_depth: u8,
    pub distance_from_selected: usize,
    pub edge_hit_top: bool,
    pub edge_hit_bottom: bool,
    pub momentum_signed: i8,
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

    row = row.child(motion_badge(
        cues.momentum_signed,
        cues.edge_hit_top,
        cues.edge_hit_bottom,
    ));

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
        .bg(rgb(current_palette().bg_badge))
        .text_color(rgb(current_palette().text_dim))
        .text_size(px(12.))
        .child(label)
}

fn motion_badge(momentum_signed: i8, edge_hit_top: bool, edge_hit_bottom: bool) -> Div {
    if momentum_signed < 0 {
        return motion_badge_element(if edge_hit_top { "^ edge" } else { "^" }, momentum_signed);
    }
    if momentum_signed > 0 {
        return motion_badge_element(
            if edge_hit_bottom { "v edge" } else { "v" },
            momentum_signed,
        );
    }
    motion_badge_placeholder()
}

fn motion_badge_element(label: &'static str, momentum_signed: i8) -> Div {
    div()
        .h(px(18.))
        .w(px(48.))
        .flex()
        .items_center()
        .justify_center()
        .bg(momentum_badge_bg(momentum_signed))
        .text_color(rgb(current_palette().text_selected))
        .text_size(px(10.))
        .child(label)
}

fn motion_badge_placeholder() -> Div {
    div().h(px(18.)).w(px(48.))
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
            rgb(current_palette().text_dim)
        } else {
            rgb(current_palette().text_faint)
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
        .bg(rgb(current_palette().bg_badge))
        .text_color(rgb(current_palette().text_dim))
        .text_size(px(10.))
        .child("gap")
}

fn boost_badge(boost: i32, selected: bool) -> Div {
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(if selected {
            rgb(current_palette().boost_bg)
        } else {
            rgb(current_palette().bg)
        })
        .text_color(if selected {
            rgb(current_palette().text_selected)
        } else {
            rgb(current_palette().text_faint)
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
            rgb(current_palette().semantic_freq)
        } else {
            match kind {
                MatchKind::Prefix => rgb(current_palette().semantic_prefix),
                MatchKind::Contains => rgb(current_palette().semantic_contains),
                MatchKind::Fuzzy => rgb(current_palette().semantic_fuzzy),
            }
        }
    } else {
        rgb(current_palette().bg)
    };
    div()
        .h(px(18.))
        .px_2()
        .flex()
        .items_center()
        .bg(bg)
        .text_color(if selected {
            rgb(current_palette().text_selected)
        } else {
            rgb(current_palette().text_faint)
        })
        .text_size(px(10.))
        .child(label)
}

fn momentum_badge_bg(momentum_signed: i8) -> gpui::Rgba {
    match momentum_signed {
        -5..=-1 => match momentum_signed.abs() {
            1 => rgb(current_palette().momentum_up[0]),
            2 => rgb(current_palette().momentum_up[1]),
            3 => rgb(current_palette().momentum_up[2]),
            4 => rgb(current_palette().momentum_up[3]),
            _ => rgb(current_palette().momentum_up[4]),
        },
        1..=5 => match momentum_signed {
            1 => rgb(current_palette().momentum_down[0]),
            2 => rgb(current_palette().momentum_down[1]),
            3 => rgb(current_palette().momentum_down[2]),
            4 => rgb(current_palette().momentum_down[3]),
            _ => rgb(current_palette().momentum_down[4]),
        },
        _ => rgb(current_palette().bg_selected),
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
        rgb(current_palette().bg_edge)
    } else if hidden_count <= 3 {
        if is_up {
            rgb(current_palette().compass_up[0])
        } else {
            rgb(current_palette().compass_down[0])
        }
    } else if hidden_count <= 7 {
        if is_up {
            rgb(current_palette().compass_up[1])
        } else {
            rgb(current_palette().compass_down[1])
        }
    } else if is_up {
        rgb(current_palette().compass_up[2])
    } else {
        rgb(current_palette().compass_down[2])
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
        rgb(current_palette().highlight_hot)
    } else if contiguous_left || contiguous_right {
        rgb(current_palette().highlight_warm)
    } else {
        rgb(current_palette().highlight_cool)
    }
}

fn row_text_color(selected: bool, previous_selected: bool, trail_depth: u8) -> gpui::Rgba {
    if selected {
        rgb(current_palette().text_selected)
    } else if trail_depth >= 2 {
        rgb(current_palette().text_dim)
    } else if trail_depth == 1 || previous_selected {
        rgb(current_palette().text_muted)
    } else {
        rgb(current_palette().text_faint)
    }
}

fn row_bg_color(
    selected: bool,
    previous_selected: bool,
    trail_depth: u8,
    distance_from_selected: usize,
) -> gpui::Rgba {
    if selected {
        rgb(current_palette().bg_selected)
    } else if trail_depth >= 2 {
        rgb(current_palette().bg_trail_hot)
    } else if trail_depth == 1 {
        rgb(current_palette().bg_trail)
    } else if previous_selected {
        rgb(current_palette().bg_near)
    } else if distance_from_selected <= 1 {
        rgb(current_palette().bg_edge)
    } else {
        rgb(current_palette().bg)
    }
}

pub fn bg_color() -> gpui::Rgba {
    rgb(current_palette().bg)
}
