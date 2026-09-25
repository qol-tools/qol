use std::ops::Range;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::text::shaped_width;
use qol_gpui::text_edit::{self, CaretStyle, TextField, TextFieldElement};
use qol_gpui::theme::{
    launcher_runtime, LauncherPalette, RADIUS_CARD, RADIUS_TIGHT, TEXT_BODY, TEXT_MICRO, TEXT_NANO,
    TEXT_TITLE,
};
use qol_gpui::trail::{Trail, TrailItem};

use super::layout::{FLOW_ROW_HEIGHT, HEADER_HEIGHT, WINDOW_WIDTH};
use super::menu::MenuKind;
use super::state::TrailFocus;
use super::LauncherView;
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

pub struct SearchBarStatus {
    pub mode: Option<SearchMode>,
    pub help_open: bool,
    pub pending: bool,
    pub list_focused: bool,
}

pub fn search_bar(
    field: &TextField,
    launch_error: Option<&str>,
    status: SearchBarStatus,
    placeholder: &str,
    window: &mut gpui::Window,
    cx: &mut Context<LauncherView>,
) -> Div {
    let kit = qol_gpui::kit::kit();
    let chevron_font = window.text_style().font().bold();
    let mono_font = font(qol_gpui::theme::font_mono());
    let mono_advance = shaped_width(window, "0", mono_font.clone(), TEXT_BODY);
    let trailing = if status.mode.is_some() { 94.0 } else { 40.0 };
    let chevron_width = shaped_width(window, "\u{203A}", chevron_font, TEXT_BODY);
    let visible = text_edit::visible_char_count(
        WINDOW_WIDTH
            - 2.0 * qol_gpui::theme::SPACE_PAD
            - chevron_width
            - 2.0 * 10.0
            - trailing
            - CARET_WIDTH,
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
        .border_color(rgba(kit.washes.hairline.packed()))
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
                        .text_color(rgb(current_palette().text))
                        .flex()
                        .items_center()
                        .child({
                            let element = TextFieldElement::new(field, visible, mono_advance)
                                .selection(
                                    rgb(current_palette().bg_selected).into(),
                                    Some(rgb(current_palette().text).into()),
                                );
                            let element = if status.list_focused {
                                element
                            } else {
                                element.caret(CaretStyle {
                                    color: rgb(current_palette().highlight).into(),
                                    width: CARET_WIDTH,
                                    height: 16.0,
                                    top: 1.0,
                                    radius: 1.0,
                                })
                            };
                            element
                                .placeholder(
                                    placeholder.to_owned(),
                                    rgb(current_palette().text_muted).into(),
                                )
                                .render()
                        }),
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
        .when_some(status.mode, |bar, mode| {
            bar.child(
                kit.chip(mode.label(), kit.palette.accent_ink)
                    .id("launcher-search-mode")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.set_search_mode(this.state.mode.next(), cx);
                    })),
            )
        })
        .when(status.pending, |bar| {
            bar.child(
                qol_gpui::Busy::ring("flow-pending", rgb(kit.palette.accent_ink))
                    .size(px(TEXT_BODY)),
            )
        })
        .child(
            div()
                .id("launcher-help-trigger")
                .flex_none()
                .h(px(qol_gpui::theme::HEIGHT_INLINE))
                .px(px(qol_gpui::theme::SPACE_SNUG))
                .flex()
                .items_center()
                .gap(px(qol_gpui::theme::SPACE_TIGHT))
                .rounded(px(RADIUS_TIGHT))
                .border(px(1.0))
                .border_color(rgba(if status.help_open {
                    kit.washes.accent_border.packed()
                } else {
                    kit.washes.hairline_strong.packed()
                }))
                .bg(rgba(kit.washes.fill_resting.packed()))
                .hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
                .cursor_pointer()
                .text_color(rgb(kit.palette.text_secondary))
                .text_size(px(TEXT_MICRO))
                .child("\u{2026}")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_menu(MenuKind::Help, cx);
                })),
        )
}

const CARET_WIDTH: f32 = 2.0;

pub fn search_placeholder(mode: SearchMode) -> &'static str {
    match mode.next() {
        SearchMode::Files => "search \u{b7} tab files \u{b7} alt+h keys",
        SearchMode::Apps => "search \u{b7} tab apps \u{b7} alt+h keys",
    }
}

pub fn row_action(
    source: ResultSource,
    held: &Modifiers,
    list_focused: bool,
) -> (&'static str, &'static str) {
    let secondary = held.secondary();
    if held.shift
        && !secondary
        && !held.alt
        && matches!(source, ResultSource::App | ResultSource::File)
    {
        return ("Shift+\u{21b5}", "open folder");
    }
    if held.alt && !held.shift {
        return ("Alt+\u{21b5}", "options");
    }
    if list_focused && secondary && !held.shift && matches!(source, ResultSource::App) {
        return ("Ctrl+\u{2192}", "raise rank");
    }
    ("\u{21b5}", "open")
}

pub fn match_percent(score: i32, best: i32, worst: i32) -> u8 {
    let span = i64::from(worst) - i64::from(best);
    if span <= 0 {
        return 100;
    }
    let distance = i64::from(worst) - i64::from(score);
    ((distance * 100 + span / 2) / span).clamp(0, 100) as u8
}

pub fn result_row(
    scored: &Scored,
    name: &str,
    selected: bool,
    match_score: u8,
    row_height: f32,
    held: &Modifiers,
    list_focused: bool,
) -> Div {
    let kit = qol_gpui::kit::kit();
    let band = qol_gpui::theme::settings_panel_runtime().grounds.band;
    let positions = &scored.m.positions;
    let highlights = if selected && !positions.is_empty() {
        char_highlights(name, positions, band.ink)
    } else {
        vec![]
    };
    let styled_name =
        StyledText::new(SharedString::from(name.to_owned())).with_highlights(highlights);
    let mut row = div()
        .flex_none()
        .h(px(row_height))
        .px(px(qol_gpui::theme::SPACE_PAD + qol_gpui::theme::SPACE_INSET))
        .flex()
        .items_center()
        .gap(px(12.0))
        .map(|row| {
            if selected {
                row.bg(rgb(band.bg))
            } else {
                row.hover(|style| style.bg(rgba(kit.washes.fill_hover.packed())))
            }
        })
        .child(kit.letter_tile(name))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_color(rgb(if selected {
                    band.ink
                } else {
                    current_palette().text_muted
                }))
                .text_size(px(qol_gpui::theme::TEXT_CAPTION))
                .child(styled_name),
        );
    if selected {
        let (shortcut, word) = row_action(scored.source, held, list_focused);
        row = row.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(qol_gpui::theme::SPACE_SNUG))
                .text_color(rgb(band.soft))
                .text_size(px(TEXT_NANO))
                .child(
                    kit.keycap(super::menu::display_shortcut(shortcut))
                        .border_color(rgba(band.edge.packed()))
                        .text_color(rgb(band.soft)),
                )
                .child(word),
        );
        let mut score = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(qol_gpui::theme::SPACE_SNUG))
            .font_family(SharedString::from(qol_gpui::theme::font_mono()))
            .text_color(rgb(band.soft))
            .text_size(px(TEXT_NANO))
            .child("match")
            .child(
                div()
                    .text_color(rgb(band.ink))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(match_score.to_string()),
            );
        if scored.manual_boost > 0 {
            score = score.child(
                div()
                    .text_color(rgb(band.ink))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("+{}", scored.manual_boost)),
            );
        }
        row = row.child(score);
    }
    if !selected && matches!(scored.source, ResultSource::Flow) {
        row = row.child(
            div()
                .flex_none()
                .font_family(SharedString::from(qol_gpui::theme::font_mono()))
                .text_color(rgb(kit.palette.text_secondary))
                .text_size(px(TEXT_NANO))
                .child("flow"),
        );
    }
    row
}

pub fn trail_body(
    kit: &qol_gpui::kit::Kit,
    rows: &[FlowRow],
    focus: TrailFocus,
    verdict: FlowVerdict,
) -> AnyElement {
    let vague = matches!(verdict, FlowVerdict::Vague | FlowVerdict::Checking);
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
            .child(vague_fence(kit, verdict == FlowVerdict::Checking))
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

fn vague_fence(kit: &qol_gpui::kit::Kit, checking: bool) -> Div {
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
                .child(if checking {
                    "CHECKING ANSWER · RELATED MEMORIES"
                } else {
                    "NO CONFIDENT ANSWER · RELATED MEMORIES"
                }),
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

fn char_highlights(
    name: &str,
    positions: &[usize],
    ink: u32,
) -> Vec<(Range<usize>, HighlightStyle)> {
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
                    color: Some(rgb(ink).into()),
                    font_weight: Some(FontWeight::BOLD),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.5),
                        color: Some(rgb(ink).into()),
                        wavy: false,
                    }),
                    ..Default::default()
                },
            ))
        })
        .collect()
}

pub fn bg_color() -> gpui::Rgba {
    rgb(current_palette().bg)
}

#[cfg(test)]
mod tests {
    use super::{answer_lead, match_percent, row_action, search_placeholder};
    use crate::discovery::search::{ResultSource, SearchMode};
    use crate::flow::FlowRow;
    use crate::ui::input::InputEffect;
    use crate::ui::state::LauncherState;
    use gpui::Modifiers;

    #[test]
    fn empty_line_names_the_mode_tab_switches_to() {
        assert_eq!(
            search_placeholder(SearchMode::Apps),
            "search \u{b7} tab files \u{b7} alt+h keys"
        );
        assert_eq!(
            search_placeholder(SearchMode::Files),
            "search \u{b7} tab apps \u{b7} alt+h keys"
        );
    }

    #[test]
    fn chosen_row_names_what_the_held_keys_do() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::none()
        };
        let secondary = Modifiers::secondary_key();
        assert_eq!(
            row_action(ResultSource::App, &Modifiers::none(), true),
            ("\u{21b5}", "open")
        );
        assert_eq!(
            row_action(ResultSource::File, &shift, true),
            ("Shift+\u{21b5}", "open folder")
        );
        assert_eq!(
            row_action(ResultSource::Flow, &shift, true),
            ("\u{21b5}", "open")
        );
        assert_eq!(
            row_action(ResultSource::File, &alt, true),
            ("Alt+\u{21b5}", "options")
        );
        assert_eq!(
            row_action(ResultSource::App, &secondary, true),
            ("Ctrl+\u{2192}", "raise rank")
        );
        assert_eq!(
            row_action(ResultSource::App, &secondary, false),
            ("\u{21b5}", "open"),
            "the search box keeps Ctrl+arrows for words"
        );
        assert_eq!(
            row_action(ResultSource::File, &secondary, true),
            ("\u{21b5}", "open")
        );
        assert_eq!(
            LauncherState::new().apply_key("enter", &shift, 1),
            InputEffect::OpenFolder
        );
        let mut in_list = LauncherState::new();
        in_list.list_focused = true;
        assert_eq!(
            in_list.apply_key("right", &secondary, 1),
            InputEffect::BoostUp
        );
    }

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
    fn selected_score_uses_total_order_and_handles_ties() {
        assert_eq!(match_percent(-100, -100, 100), 100);
        assert_eq!(match_percent(0, -100, 100), 50);
        assert_eq!(match_percent(100, -100, 100), 0);
        assert_eq!(match_percent(42, 42, 42), 100);
        assert_eq!(match_percent(i32::MIN, i32::MIN, i32::MAX), 100);
        assert_eq!(match_percent(i32::MAX, i32::MIN, i32::MAX), 0);
    }
}
