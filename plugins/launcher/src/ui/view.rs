use std::path::Path;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::icon::{icon, Icon};
use qol_gpui::kit::Chip;
use qol_gpui::text::shaped_width;
use qol_gpui::text::{cased, TextStyled};
use qol_gpui::text_edit::{self, CaretStyle, TextField, TextFieldElement};
use qol_gpui::theme::{
    translucent, Alpha, TextStyle, FOCUS_RING_EDGE, FOCUS_RING_HALO, LINE, RADIUS_CARD,
    RADIUS_CONTROL, RADIUS_THUMB, RADIUS_TIGHT, SPACE_CELL, SPACE_INSET, SPACE_PAD, SPACE_SNUG,
    SPACE_TIGHT, TEXT_BODY, TEXT_MICRO,
};
use qol_gpui::trail::{Trail, TrailItem};
use qol_gpui::Key;

use super::feedback::{self, Feedback};
use super::layout::{FLOW_ROW_HEIGHT, HEADER_HEIGHT};
use super::sizing::AppCardSize;
use super::state::TrailFocus;
use super::LauncherView;
use crate::discovery::search::SearchMode;
use crate::flow::{host_of, lead_of, sources_of, FlowEntry, FlowRow, FlowVerdict};

pub const CARD_HEIGHT: f32 = 116.0;
// The card is a filled box in a trail slot whose rows start one PAD_TOP down,
// so it has to end one PAD_TOP short of the next slot to keep that rhythm.
const CARD_GAP: f32 = qol_gpui::trail::motion::PAD_TOP;
const CARD_DOT_CY: f32 = 36.0;
const FIELD_FILL_ALPHA: Alpha = Alpha::Trace;
const MODE_FILL_ALPHA: Alpha = Alpha::Halo;
const MODE_INK_MIX: f32 = 0.6;
const MODE_KEY_MIX: f32 = 0.3;
const MODE_KEY_EDGE_ALPHA: Alpha = Alpha::Veil;
const MODE_CHIP_HEIGHT: f32 = 24.0;
const MODE_TRAILING: f32 = 120.0;
const CHOSEN_CARD_MIX: f32 = 0.31;
const PANEL_TINT_TYPING: f32 = 0.06;
const PANEL_TINT_RESULTS: f32 = 0.14;
const LINE_INK_MIX: f32 = 0.86;
const BADGE_HEIGHT: f32 = 22.0;
const BADGE_ALPHA: Alpha = Alpha::Halo;
pub const APP_GROW: f32 = 8.0;

pub struct SearchBarStatus<'a> {
    pub mode: Option<SearchMode>,
    pub pending: bool,
    pub list_focused: bool,
    pub panel_active: bool,
    pub feedback: Option<&'a Feedback>,
}

pub fn search_bar(
    field: &TextField,
    launch_error: Option<&str>,
    status: SearchBarStatus<'_>,
    placeholder: &str,
    width: f32,
    window: &mut gpui::Window,
    cx: &mut Context<LauncherView>,
) -> Div {
    let kit = qol_gpui::kit::kit();
    let pane = kit.grounds.pane;
    let mono_font = font(qol_gpui::theme::font_mono());
    let mono_advance = shaped_width(window, "0", mono_font.clone(), TextStyle::Code.spec().size);
    let trailing = if status.mode.is_some() {
        MODE_TRAILING
    } else {
        0.0
    };
    let visible = text_edit::visible_char_count(
        width
            - 2.0 * SPACE_INSET
            - SPACE_CELL
            - SPACE_INSET
            - TEXT_BODY
            - 2.0 * SPACE_INSET
            - trailing
            - CARET_WIDTH,
        mono_advance,
    );
    let spoken = status
        .feedback
        .and_then(|feedback| feedback.cue.line().map(|text| (feedback, text)));
    let chip_fade = spoken
        .as_ref()
        .map(|(feedback, _)| feedback.id("launcher-chip-fade"));
    let line = spoken.map(|(feedback, text)| (text, feedback.id("launcher-line")));
    div()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .pt(px(SPACE_INSET))
        .pb(px(SPACE_SNUG))
        .px(px(SPACE_INSET))
        .bg(rgb(pane.bg))
        .child(
            div()
                .flex_1()
                .h_full()
                .min_w(px(0.0))
                .relative()
                .flex()
                .items_center()
                .gap(px(SPACE_INSET))
                .pl(px(SPACE_CELL))
                .pr(px(SPACE_INSET))
                .rounded(px(RADIUS_CARD))
                .bg(rgba(translucent(pane.ink, FIELD_FILL_ALPHA)))
                .border(px(LINE))
                .border_color(rgba(kit.washes.hairline_strong.packed()))
                .when(!status.list_focused, |field| field.children(focus_ring()))
                .child(icon(Icon::Prompt, TEXT_BODY, kit.palette.accent_ink))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .child(
                            div()
                                .h(px(18.))
                                .overflow_hidden()
                                .text(TextStyle::Code)
                                .text_color(rgb(pane.ink))
                                .flex()
                                .items_center()
                                .child({
                                    let element =
                                        TextFieldElement::new(field, visible, mono_advance)
                                            .selection(
                                                rgb(kit.grounds.band.bg).into(),
                                                Some(rgb(pane.soft).into()),
                                            );
                                    let element = if status.list_focused {
                                        element
                                    } else {
                                        element.caret(CaretStyle {
                                            color: rgb(kit.palette.accent_ink).into(),
                                            width: CARET_WIDTH,
                                            height: 16.0,
                                            top: 1.0,
                                            radius: 1.0,
                                        })
                                    };
                                    element
                                        .placeholder(placeholder.to_owned(), rgb(pane.faint).into())
                                        .render()
                                }),
                        )
                        .when_some(launch_error, |field, error| {
                            field.child(
                                div()
                                    .text(TextStyle::Detail)
                                    .text_color(rgb(kit.palette.warning_ink))
                                    .line_clamp(1)
                                    .child(error.to_owned()),
                            )
                        }),
                )
                .when(status.pending, |bar| {
                    bar.child(
                        qol_gpui::Busy::ring("flow-pending", rgb(kit.palette.accent_ink))
                            .size(px(TEXT_BODY)),
                    )
                })
                .when_some(status.mode, |bar, mode| {
                    let chip = mode_chip(mode, status.panel_active, cx);
                    match chip_fade {
                        Some(id) => bar.child(chip.with_animation(
                            id,
                            feedback::held(feedback::LINE_HOLD),
                            |chip, delta| {
                                chip.opacity(
                                    1.0 - feedback::fade_in_hold_fade(feedback::LINE_HOLD, delta),
                                )
                            },
                        )),
                        None => bar.child(chip),
                    }
                })
                .when_some(line, |bar, (text, id)| {
                    bar.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(SPACE_INSET))
                            .flex()
                            .items_center()
                            .text(TextStyle::Detail)
                            .text_color(rgb(qol_color::mix_rgb(pane.bg, pane.ink, LINE_INK_MIX)))
                            .child(text)
                            .with_animation(
                                id,
                                feedback::held(feedback::LINE_HOLD),
                                |line, delta| {
                                    line.opacity(feedback::fade_in_hold_fade(
                                        feedback::LINE_HOLD,
                                        delta,
                                    ))
                                },
                            ),
                    )
                }),
        )
}

fn mode_chip(
    mode: SearchMode,
    panel_active: bool,
    cx: &mut Context<LauncherView>,
) -> Stateful<Div> {
    let kit = qol_gpui::kit::kit();
    let accent = kit.palette.accent;
    let ink = qol_color::mix_rgb(accent, kit.grounds.pane.ink, MODE_INK_MIX);
    let key_ink = qol_color::mix_rgb(accent, kit.grounds.pane.ink, MODE_KEY_MIX);
    div()
        .id("launcher-search-mode")
        .flex_none()
        .h(px(MODE_CHIP_HEIGHT))
        .px(px(SPACE_INSET))
        .flex()
        .items_center()
        .gap(px(SPACE_SNUG))
        .rounded(px(RADIUS_CONTROL))
        .bg(rgba(translucent(accent, MODE_FILL_ALPHA)))
        .text(TextStyle::Label)
        .text_color(rgb(ink))
        .cursor_pointer()
        .child(icon(
            match mode {
                SearchMode::Apps => Icon::Grid,
                SearchMode::Files => Icon::Folder,
            },
            TextStyle::Label.spec().size,
            ink,
        ))
        .child(cased(TextStyle::Label, mode.label()))
        .when(!panel_active, |chip| {
            chip.child(
                div()
                    .flex_none()
                    .px(px(SPACE_TIGHT))
                    .rounded(px(RADIUS_THUMB))
                    .border(px(LINE))
                    .border_color(rgba(translucent(key_ink, MODE_KEY_EDGE_ALPHA)))
                    .child(kit.key_name(&[Key::TAB], key_ink)),
            )
        })
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.set_search_mode(this.state.mode.next(), cx);
        }))
}

const CARET_WIDTH: f32 = 2.0;

pub fn search_placeholder(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Apps => "search \u{b7} alt+h keys",
        SearchMode::Files => "search files \u{b7} alt+h keys",
    }
}

pub struct AppCard<'a> {
    pub name: &'a str,
    pub summary: Option<&'a str>,
    pub art: Option<&'a Path>,
    pub chosen: bool,
    pub lit: f32,
    pub cue: f32,
    pub size: AppCardSize,
    pub feedback: Option<&'a Feedback>,
}

pub fn app_card(card: AppCard<'_>) -> Div {
    let kit = qol_gpui::kit::kit();
    let pane = kit.grounds.pane;
    let mine = card
        .feedback
        .filter(|feedback| feedback.is_about(card.name));
    let well = if card.chosen {
        kit.grounds.band.well
    } else {
        pane.well
    };
    div()
        .flex_none()
        .w_full()
        .h(px(card.size.height))
        .px(px(SPACE_PAD))
        .flex()
        .items_center()
        .gap(px(SPACE_CELL))
        .rounded(px(RADIUS_CARD))
        .bg(rgb(card_ground(card.lit)))
        .shadow(card_cast(card.lit))
        .child({
            let name = card.name.to_owned();
            let art = card.art.map(Path::to_path_buf);
            let size = card.size;
            growing(
                mine.filter(|feedback| feedback.cue.grows()),
                APP_GROW,
                move |extra| {
                    tile(
                        &name,
                        art.as_deref(),
                        size.tile + extra,
                        size.icon + extra,
                        size.radius,
                        well.packed(),
                        pane.soft,
                    )
                },
            )
        })
        .child(
            div()
                .flex_grow()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(SPACE_SNUG))
                .child(name_text(card.name, card.size.name, pane.ink))
                .when_some(card.summary, |text, summary| {
                    text.child(
                        div()
                            .text(TextStyle::Detail)
                            .text_color(rgb(if card.chosen {
                                kit.grounds.band.soft
                            } else {
                                pane.faint
                            }))
                            .line_clamp(1)
                            .child(summary.to_owned()),
                    )
                }),
        )
        .when(card.cue > 0.0, |row| row.child(enter_cue(card.cue)))
        .when_some(
            mine.and_then(|feedback| feedback.cue.badge().map(|text| (feedback, text))),
            |row, (feedback, text)| {
                row.child(
                    div()
                        .flex_none()
                        .h(px(BADGE_HEIGHT))
                        .px(px(SPACE_INSET))
                        .flex()
                        .items_center()
                        .rounded(px(RADIUS_CONTROL))
                        .bg(rgba(translucent(pane.ink, BADGE_ALPHA)))
                        .text(TextStyle::Detail)
                        .gap(px(SPACE_TIGHT))
                        .text_color(rgb(pane.ink))
                        .children(
                            feedback
                                .cue
                                .arrow()
                                .map(|arrow| icon(arrow, TEXT_MICRO, pane.ink)),
                        )
                        .child(text)
                        .with_animation(
                            feedback.id("launcher-badge"),
                            feedback::held(feedback::BADGE_HOLD),
                            |badge, delta| {
                                badge.opacity(feedback::fade_in_hold_fade(
                                    feedback::BADGE_HOLD,
                                    delta,
                                ))
                            },
                        ),
                )
            },
        )
}

pub fn growing<F>(grow: Option<&Feedback>, by: f32, build: F) -> AnyElement
where
    F: Fn(f32) -> Div + 'static,
{
    let Some(feedback) = grow else {
        return build(0.0).into_any_element();
    };
    div()
        .flex_none()
        .with_animation(
            feedback.id("launcher-grow"),
            qol_gpui::motion::animation(qol_gpui::theme::Motion::SETTLE),
            move |frame, delta| frame.child(build(by * delta)),
        )
        .into_any_element()
}

pub fn enter_cue(opacity: f32) -> Div {
    let kit = qol_gpui::kit::kit();
    div()
        .flex_none()
        .opacity(opacity)
        .child(kit.keycap_inked(Key::ENTER, kit.grounds.pane.faint))
}

pub fn card_ground(lit: f32) -> u32 {
    let kit = qol_gpui::kit::kit();
    qol_color::mix_rgb(
        kit.grounds.menu.bg,
        qol_color::mix_rgb(
            kit.palette.accent_fill_base,
            kit.palette.accent,
            CHOSEN_CARD_MIX,
        ),
        lit,
    )
}

fn focus_ring() -> [Div; 2] {
    let pane = qol_gpui::kit::kit().grounds.pane;
    let band = |reach: f32, width: f32, color: Rgba| {
        div()
            .absolute()
            .top(px(-reach))
            .bottom(px(-reach))
            .left(px(-reach))
            .right(px(-reach))
            .rounded(px(RADIUS_CARD + reach - LINE))
            .border(px(width))
            .border_color(color)
    };
    [
        band(
            LINE + FOCUS_RING_HALO,
            FOCUS_RING_HALO - FOCUS_RING_EDGE,
            rgba(pane.halo.packed()),
        ),
        band(LINE + FOCUS_RING_EDGE, FOCUS_RING_EDGE, rgb(pane.mark)),
    ]
}

pub fn panel_fill(lit: f32) -> u32 {
    let kit = qol_gpui::kit::kit();
    qol_color::mix_rgb(
        kit.grounds.menu.bg,
        kit.palette.accent,
        PANEL_TINT_TYPING + (PANEL_TINT_RESULTS - PANEL_TINT_TYPING) * lit,
    )
}

pub fn card_cast(lit: f32) -> Vec<BoxShadow> {
    let dark = 1.0 - lit.clamp(0.0, 1.0);
    qol_gpui::kit::raised_shadow(qol_gpui::kit::kit().washes.cast.rgb)
        .into_iter()
        .map(|mut layer| {
            layer.color.a *= dark;
            layer
        })
        .collect()
}

pub fn tile(
    name: &str,
    art: Option<&Path>,
    size: f32,
    art_size: f32,
    radius: f32,
    well: u32,
    ink: u32,
) -> Div {
    let frame = div()
        .flex_none()
        .size(px(size))
        .rounded(px(radius))
        .bg(rgba(well))
        .flex()
        .items_center()
        .justify_center();
    match art {
        Some(art) => frame.child(img(art.to_path_buf()).size(px(art_size))),
        None => frame
            .text(super::sizing::name_style((art_size * 0.6).round()))
            .text_color(rgb(ink))
            .child(
                name.chars()
                    .find(|character| character.is_alphanumeric())
                    .map_or_else(
                        || "\u{2022}".to_owned(),
                        |character| character.to_uppercase().to_string(),
                    ),
            ),
    }
}

pub fn name_text(name: &str, style: TextStyle, ink: u32) -> Div {
    div()
        .text(style)
        .text_color(rgb(ink))
        .child(name.to_owned())
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
        .border(px(qol_gpui::theme::LINE))
        .border_color(rgba(kit.washes.hairline.packed()))
        .bg(rgb(kit.palette.surface_raised))
        .py(px(qol_gpui::theme::SPACE_INSET))
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
                .text(TextStyle::Heading)
                .wraps()
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
                    .text(TextStyle::Detail)
                    .line_clamp(3)
                    .child(explanation),
            );
        }
    } else if let Some(copy) = row.copy.clone().or_else(|| Some(row.title.clone())) {
        card = card.child(
            div()
                .text_color(rgb(kit.palette.text_primary))
                .text(TextStyle::Detail)
                .line_clamp(3)
                .child(copy),
        );
    }
    let mut meta = div().mt(px(4.0)).flex().items_center().gap(px(8.0));
    meta = meta.child(kit.chip(Chip::Tag("true now".into()), kit.grounds.pane));
    if let Some(sources) = sources_of(&row.raw).filter(|count| *count >= 2) {
        meta = meta.child(
            div()
                .text_color(rgb(kit.palette.text_muted))
                .text(TextStyle::Detail)
                .child(format!("{sources} sources agree")),
        );
    }
    let at = nodes.first().map(|node| node.at.as_str()).unwrap_or("");
    if !at.is_empty() {
        meta = meta.child(
            div()
                .text(TextStyle::Code)
                .text_color(rgb(kit.palette.text_muted))
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
        .px(px(qol_gpui::theme::SPACE_SNUG))
        .rounded(px(RADIUS_TIGHT))
        .border(px(qol_gpui::theme::LINE))
        .border_color(rgba(kit.washes.hairline_strong.packed()))
        .text(TextStyle::Code)
        .text_color(rgb(kit.palette.text_muted))
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
        .gap(px(qol_gpui::theme::SPACE_INSET))
        .px(px(qol_gpui::theme::SPACE_PAD))
        .child(
            div()
                .flex_none()
                .text(TextStyle::Label)
                .text_color(rgb(kit.palette.text_secondary))
                .child(if checking {
                    "CHECKING ANSWER · RELATED MEMORIES"
                } else {
                    "NO CONFIDENT ANSWER · RELATED MEMORIES"
                }),
        )
        .child(div().flex_1().h(px(1.0)).bg(rgb(kit.palette.border_subtle)))
}

pub fn flow_empty_state(kit: &qol_gpui::kit::Kit) -> Div {
    kit.empty("no memory covers this", None)
}

pub fn detail_body(
    kit: &qol_gpui::kit::Kit,
    row: &FlowRow,
    height: f32,
    scroll: &ScrollHandle,
) -> Div {
    let text = row.copy.clone().unwrap_or_else(|| row.title.clone());
    let detail = crate::flow::detail_of(&row.raw);
    let mut fields = div().flex().flex_col().gap(px(qol_gpui::theme::SPACE_SNUG));
    for (label, value) in &detail {
        fields = fields.child(
            div()
                .flex()
                .gap(px(qol_gpui::theme::SPACE_INSET))
                .child(
                    div()
                        .w(px(92.0))
                        .flex_none()
                        .text_color(rgb(kit.palette.text_muted))
                        .text(TextStyle::Label)
                        .child(cased(TextStyle::Label, label)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_color(rgb(kit.palette.text_secondary))
                        .text(TextStyle::Detail)
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
                .p(px(qol_gpui::theme::SPACE_CELL))
                .pt(px(16.0))
                .gap(px(qol_gpui::theme::SPACE_CELL))
                .child(
                    div()
                        .text_color(rgb(kit.palette.text_primary))
                        .text(TextStyle::Detail)
                        .wraps()
                        .child(text),
                )
                .when(!detail.is_empty(), |body| body.child(fields)),
        )
        .child(kit.scroll_cue(
            qol_gpui::scrollbar::ScrollSource::Handle {
                handle: scroll.clone(),
                children: 1 + usize::from(!detail.is_empty()),
            },
            kit.grounds.pane,
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
        .child(kit.hint(Key::ENTER, enter_label.to_owned()))
        .child(kit.hint(Key::DOWN, "back in time"))
        .child(kit.hint(Key::UP, "forward"))
        .child(kit.hint(Key::ESC, "back"))
        .child(kit.chip(Chip::Tag(entry.title.clone().into()), kit.grounds.pane))
        .child(div().flex_1())
}

pub fn hint_bar_detail() -> Div {
    let kit = qol_gpui::kit::kit();
    kit.hint_bar()
        .child(kit.hint(Key::ENTER, "copy"))
        .child(kit.hint(Key::UP_DOWN, "scroll"))
        .child(kit.hint(Key::ESC, "back"))
        .child(div().flex_1())
}

pub fn bg_color() -> gpui::Rgba {
    rgb(qol_gpui::kit::kit().grounds.pane.bg)
}

#[cfg(test)]
mod tests {
    use super::{answer_lead, search_placeholder};
    use crate::discovery::search::SearchMode;
    use crate::flow::FlowRow;
    use crate::ui::input::InputEffect;
    use crate::ui::state::LauncherState;
    use gpui::Modifiers;

    #[test]
    fn empty_line_names_the_search_and_the_key_list() {
        assert_eq!(
            search_placeholder(SearchMode::Apps),
            "search \u{b7} alt+h keys"
        );
        assert_eq!(
            search_placeholder(SearchMode::Files),
            "search files \u{b7} alt+h keys"
        );
    }

    #[test]
    fn held_keys_still_open_folders_and_raise_rank() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        let secondary = Modifiers::secondary_key();
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
}
