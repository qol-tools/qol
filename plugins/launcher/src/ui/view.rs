use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::hint_bar::{estimated_chip_width, fit_hints, BarItem, HintDescriptor};
use qol_gpui::theme::{
    launcher_runtime, LauncherPalette, RADIUS_CARD, RADIUS_TIGHT, TEXT_BODY, TEXT_MICRO, TEXT_NANO,
    TEXT_TITLE,
};
use qol_gpui::trail::{Trail, TrailItem};

use super::layout::{FLOW_ROW_HEIGHT, HEADER_HEIGHT, WINDOW_WIDTH};
use super::state::TrailFocus;
use crate::discovery::search::{ResultSource, Scored, SearchMode};
use crate::flow::{host_of, lead_of, sources_of, FlowEntry, FlowRow, FlowVerdict};

pub const CARD_HEIGHT: f32 = 116.0;
// The card is a filled box in a trail slot whose rows start one PAD_TOP down,
// so it has to end one PAD_TOP short of the next slot to keep that rhythm.
const CARD_GAP: f32 = qol_gpui::trail::motion::PAD_TOP;
const CARD_DOT_CY: f32 = 36.0;

fn current_palette() -> LauncherPalette {
    launcher_runtime()
}

pub fn palette() -> LauncherPalette {
    current_palette()
}

#[allow(clippy::too_many_arguments)]
pub fn search_bar(
    query: &str,
    launch_error: Option<&str>,
    cursor: usize,
    selection: Option<(usize, usize)>,
    selected: usize,
    result_count: usize,
    pending: bool,
    placeholder: &str,
    window: &mut gpui::Window,
) -> Div {
    let kit = qol_gpui::kit::kit();
    let counter = if result_count == 0 {
        format!("0 / {result_count}")
    } else {
        format!("{} / {result_count}", selected.min(result_count - 1) + 1)
    };
    let chevron_font = window.text_style().font().bold();
    let mono_font = font(qol_gpui::theme::font_mono());
    let mono_advance = shaped_width(window, "0", mono_font.clone(), TEXT_BODY);
    let trailing = if pending {
        px(TEXT_BODY).into()
    } else {
        shaped_width(window, &counter, mono_font, TEXT_NANO)
    };
    let chevron_width = shaped_width(window, "\u{203A}", chevron_font, TEXT_BODY);
    let visible = visible_char_count(
        WINDOW_WIDTH - 2.0 * qol_gpui::theme::SPACE_PAD - chevron_width - 2.0 * 10.0 - trailing,
        mono_advance,
    );
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
                        .child(search_bar_content(
                            query,
                            cursor,
                            selection,
                            placeholder,
                            visible,
                        )),
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
        .child(if pending {
            qol_gpui::Spinner::new("flow-pending", rgb(kit.palette.accent_ink))
                .size(px(TEXT_BODY))
                .into_any_element()
        } else {
            div()
                .flex_none()
                .text_color(rgb(kit.palette.text_muted))
                .text_size(px(TEXT_NANO))
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .child(counter)
                .into_any_element()
        })
}

fn search_bar_content(
    query: &str,
    cursor: usize,
    selection: Option<(usize, usize)>,
    placeholder: &str,
    visible: usize,
) -> AnyElement {
    if query.is_empty() {
        return div()
            .text_color(rgb(current_palette().text_muted))
            .child(placeholder.to_owned())
            .into_any_element();
    }

    let char_count = query.chars().count();
    let (view_start, view_end) = search_window(char_count, cursor, visible);

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

fn shaped_width(window: &mut gpui::Window, text: &str, run_font: Font, font_size: f32) -> f32 {
    window
        .text_system()
        .shape_line(
            SharedString::from(text.to_owned()),
            px(font_size),
            &[TextRun {
                len: text.len(),
                font: run_font,
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        )
        .width
        .into()
}

fn visible_char_count(available_px: f32, advance_px: f32) -> usize {
    let fits = if advance_px > 0.0 {
        (available_px / advance_px).floor() as usize
    } else {
        0
    };
    fits.max(8)
}

fn search_window(char_count: usize, cursor: usize, visible: usize) -> (usize, usize) {
    let view_start = if char_count <= visible {
        0
    } else {
        cursor
            .saturating_sub(visible.saturating_sub(2))
            .min(char_count.saturating_sub(visible))
    };
    let view_end = (view_start + visible).min(char_count);
    (view_start, view_end)
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

pub fn trail_body(
    kit: &qol_gpui::kit::Kit,
    rows: &[FlowRow],
    focus: TrailFocus,
    verdict: FlowVerdict,
) -> AnyElement {
    let vague = verdict == FlowVerdict::Vague;
    if let Some(row) = answer_lead(vague, rows) {
        return answer_trail(kit, row, rows, focus);
    }
    let items = rows
        .iter()
        .map(|row| {
            let node = &crate::flow::trail_of(&row.raw)[0];
            let tag = if vague {
                String::new()
            } else {
                node.tag.clone()
            };
            TrailItem::new(node.at.clone(), tag, node.text.clone()).struck(node.struck)
        })
        .collect();
    let mut palette = kit.palette;
    if vague {
        palette.text_primary = palette.text_secondary;
    }
    let trail = Trail::new("flow-trail", items)
        .focus(focus.from, focus.from_index, focus.to)
        .seq(focus.seq)
        .settled(focus.settled)
        .palette(palette);
    if vague {
        div()
            .flex()
            .flex_col()
            .child(vague_fence(kit))
            .child(trail)
            .into_any_element()
    } else {
        trail.into_any_element()
    }
}

pub fn answer_lead(vague: bool, rows: &[FlowRow]) -> Option<&FlowRow> {
    (!vague)
        .then(|| rows.first())
        .flatten()
        .filter(|row| row.raw.get("kind").and_then(|value| value.as_str()) == Some("answer"))
}

fn answer_trail(
    kit: &qol_gpui::kit::Kit,
    lead_row: &FlowRow,
    rows: &[FlowRow],
    focus: TrailFocus,
) -> AnyElement {
    let nodes = crate::flow::trail_of(&lead_row.raw);
    let items = rows
        .iter()
        .map(|row| {
            let node = &crate::flow::trail_of(&row.raw)[0];
            TrailItem::new(node.at.clone(), node.tag.clone(), node.text.clone()).struck(node.struck)
        })
        .collect();
    let kit = *kit;
    let card_row = lead_row.clone();
    Trail::new("flow-trail", items)
        .head(
            move || {
                answer_card(&kit, &card_row, &nodes)
                    .h(px(CARD_HEIGHT - CARD_GAP))
                    .into_any_element()
            },
            CARD_HEIGHT,
            CARD_DOT_CY,
        )
        .focus(focus.from, focus.from_index, focus.to)
        .seq(focus.seq)
        .settled(focus.settled)
        .palette(kit.palette)
        .into_any_element()
}

fn answer_card(kit: &qol_gpui::kit::Kit, row: &FlowRow, nodes: &[crate::flow::TrailNode]) -> Div {
    let lead = lead_of(&row.raw);
    let mut card = div()
        .rounded(px(RADIUS_CARD))
        .border(px(1.0))
        .border_color(rgba(kit.washes.hairline.packed()))
        .bg(rgb(kit.palette.surface_raised))
        .py(px(10.0))
        .px(px(12.0))
        .flex()
        .flex_col()
        .gap(px(4.0));
    if let Some(lead) = &lead {
        let mut head = div().flex().items_start().gap(px(8.0)).child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(kit.palette.text_primary))
                .text_size(px(TEXT_TITLE))
                .line_height(px(24.0))
                .font_weight(FontWeight::BOLD)
                .child(lead.clone()),
        );
        if let Some(host) = host_of(&row.raw) {
            head = head.child(host_tag(kit, &host).flex_shrink_0());
        }
        card = card.child(head);
        if let Some(explanation) = row
            .copy
            .as_deref()
            .and_then(|copy| explanation_of(copy, lead))
        {
            card = card.child(
                div()
                    .text_color(rgb(kit.palette.text_secondary))
                    .text_size(px(TEXT_MICRO))
                    .line_height(px(18.0))
                    .line_clamp(3)
                    .child(explanation),
            );
        }
    } else if let Some(copy) = row.copy.clone().or_else(|| Some(row.title.clone())) {
        card = card.child(
            div()
                .text_color(rgb(kit.palette.text_primary))
                .text_size(px(TEXT_MICRO))
                .line_height(px(18.0))
                .line_clamp(3)
                .child(copy),
        );
    }
    let mut meta = div().mt(px(4.0)).flex().items_center().gap(px(8.0));
    meta = meta.child(
        kit.chip("TRUE NOW", kit.palette.accent)
            .text_size(px(TEXT_NANO)),
    );
    if let Some(sources) = sources_of(&row.raw).filter(|count| *count >= 2) {
        meta = meta.child(
            div()
                .text_color(rgb(kit.palette.text_muted))
                .text_size(px(TEXT_NANO))
                .child(format!("{sources} sources agree")),
        );
    }
    let at = nodes.first().map(|node| node.at.as_str()).unwrap_or("");
    if !at.is_empty() {
        meta = meta.child(
            div()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .text_color(rgb(kit.palette.text_muted))
                .text_size(px(TEXT_NANO))
                .child(at.to_string()),
        );
    }
    if lead.is_none() {
        if let Some(host) = host_of(&row.raw) {
            meta = meta.child(div().flex_1()).child(host_tag(kit, &host));
        }
    }
    card = card.child(meta);
    card
}

fn host_tag(kit: &qol_gpui::kit::Kit, host: &str) -> Div {
    div()
        .px(px(5.0))
        .rounded(px(RADIUS_TIGHT))
        .border(px(1.0))
        .border_color(rgba(kit.washes.hairline_strong.packed()))
        .font_family(SharedString::from(qol_gpui::theme::font_mono()))
        .text_color(rgb(kit.palette.text_muted))
        .text_size(px(TEXT_NANO))
        .child(host.to_string())
}

fn explanation_of(copy: &str, lead: &str) -> Option<String> {
    let rest = copy.strip_prefix(lead).unwrap_or(copy);
    let rest = rest.trim_start_matches([' ', '.']);
    (!rest.is_empty()).then(|| rest.to_string())
}

fn vague_fence(kit: &qol_gpui::kit::Kit) -> Div {
    div()
        .flex_none()
        .h(px(FLOW_ROW_HEIGHT))
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(qol_gpui::theme::SPACE_PAD))
        .child(
            div()
                .flex_none()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .text_color(rgb(kit.palette.text_secondary))
                .text_size(px(TEXT_NANO))
                .child("no confident answer - related memories".to_uppercase()),
        )
        .child(div().flex_1().h(px(1.0)).bg(rgb(kit.palette.border_subtle)))
}

pub fn flow_empty_state(kit: &qol_gpui::kit::Kit) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(rgb(kit.palette.text_muted))
        .text_size(px(TEXT_MICRO))
        .child("no memory covers this")
}

pub fn detail_body(
    kit: &qol_gpui::kit::Kit,
    row: &FlowRow,
    height: f32,
    scroll: &ScrollHandle,
) -> Div {
    let text = row.copy.clone().unwrap_or_else(|| row.title.clone());
    let detail = crate::flow::detail_of(&row.raw);
    let mut fields = div().flex().flex_col().gap(px(5.0));
    for (label, value) in &detail {
        fields = fields.child(
            div()
                .flex()
                .gap(px(10.0))
                .child(
                    div()
                        .w(px(92.0))
                        .flex_none()
                        .text_color(rgb(kit.palette.text_muted))
                        .text_size(px(TEXT_NANO))
                        .child(label.to_uppercase()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .text_color(rgb(kit.palette.text_secondary))
                        .text_size(px(TEXT_NANO))
                        .child(value.clone()),
                ),
        );
    }
    div()
        .relative()
        .h(px(height))
        .w_full()
        .overflow_hidden()
        .child(
            div()
                .id("flow-detail-scroll")
                .track_scroll(scroll)
                .overflow_y_scroll()
                .size_full()
                .flex()
                .flex_col()
                .p(px(14.0))
                .pt(px(16.0))
                .gap(px(14.0))
                .child(
                    div()
                        .text_color(rgb(kit.palette.text_primary))
                        .text_size(px(TEXT_MICRO))
                        .line_height(px(18.0))
                        .child(text),
                )
                .when(!detail.is_empty(), |body| body.child(fields)),
        )
        .child(qol_gpui::scrollbar::seam_track(
            scroll.clone(),
            qol_gpui::kit::alpha(kit.palette.border_subtle, 0x48),
            qol_gpui::kit::alpha(kit.palette.text_secondary, 0x8c),
        ))
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
        .child(kit.hint("\u{2193}", "back in time"))
        .child(kit.hint("\u{2191}", "forward"))
        .child(kit.hint("esc", "back"))
        .child(kit.chip(entry.title.clone(), kit.palette.accent))
        .child(div().flex_1())
}

pub fn hint_bar_detail() -> Div {
    let kit = qol_gpui::kit::kit();
    kit.hint_bar()
        .child(kit.hint("\u{23CE}", "copy"))
        .child(kit.hint("\u{2191}\u{2193}", "scroll"))
        .child(kit.hint("esc", "back"))
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

#[cfg(test)]
mod tests {
    use super::{answer_lead, search_window, visible_char_count};
    use crate::flow::FlowRow;

    fn flow_row(kind: &str) -> FlowRow {
        FlowRow {
            title: String::new(),
            subtitle: None,
            copy: None,
            raw: serde_json::json!({ "kind": kind }),
        }
    }

    #[test]
    fn trail_routes_on_a_non_vague_answer_lead_row() {
        let answer_first = [flow_row("answer"), flow_row("note")];
        assert!(answer_lead(false, &answer_first).is_some());
        assert!(answer_lead(false, &[flow_row("note"), flow_row("answer")]).is_none());
        assert!(answer_lead(true, &[flow_row("answer")]).is_none());
        assert!(answer_lead(false, &[]).is_none());
    }

    #[test]
    fn visible_char_count_floors_the_advance() {
        assert_eq!(visible_char_count(300.0, 10.0), 30);
        assert_eq!(visible_char_count(309.0, 10.0), 30);
    }

    #[test]
    fn visible_char_count_never_below_eight() {
        assert_eq!(visible_char_count(79.0, 10.0), 8);
        assert_eq!(visible_char_count(0.0, 10.0), 8);
    }

    #[test]
    fn visible_char_count_zero_advance_holds_eight() {
        assert_eq!(visible_char_count(300.0, 0.0), 8);
    }

    #[test]
    fn search_window_shows_short_query_whole() {
        assert_eq!(search_window(10, 3, 25), (0, 10));
        assert_eq!(search_window(25, 25, 25), (0, 25));
    }

    #[test]
    fn search_window_follows_the_cursor() {
        assert_eq!(search_window(60, 5, 25), (0, 25));
        assert_eq!(search_window(60, 30, 25), (7, 32));
        assert_eq!(search_window(60, 60, 25), (35, 60));
    }
}
