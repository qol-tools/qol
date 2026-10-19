mod clip;
mod control;
mod copy;
mod keys;

pub use copy::clipboard_item;
pub use keys::finishes_flow;

use gpui::prelude::FluentBuilder;
use gpui::*;
use qol_gpui::kit::Chip;
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{
    translucent, Ground, StatusTone, TextStyle, LINE, RADIUS_CARD, SPACE_CELL, SPACE_INSET,
    SPACE_PAD, SPACE_STACK, SPACE_TIGHT,
};
use qol_gpui::Key;
use qol_plugin_api::launcher_flows::{Badge, FlowEnter, Tone};

use super::about::{BAND_INSET, BAND_SHADE_ALPHA, HEAD_HEIGHT, HEAD_TILE, LINE_BLOCK};
use super::layout::{
    APP_GAP, APP_PANEL_WIDTH, FILE_INSET, LIST_PAD_Y, PANEL_GAP, PREVIEW_CARD, PREVIEW_HEIGHT,
    WINDOW_WIDTH,
};
use super::state::FlowSession;
use super::view;
use super::LauncherView;
use crate::flow::{FlowEntry, FlowRow};
use clip::{body_of, Detail, Frame};

const CARD_TILE: f32 = 38.0;
const CARD_GLYPH: f32 = 20.0;
const HEAD_GLYPH: f32 = 28.0;
const NAME_LINE: f32 = 20.0;
const FRAME_TOP: f32 = HEAD_HEIGHT + SPACE_INSET;
const KEYS_TOP: f32 = PREVIEW_HEIGHT - BAND_INSET - LINE_BLOCK;
const FACT_ROWS: usize = 4;
const FACTS_AIR: f32 = SPACE_INSET + SPACE_STACK;
const FACTS_TOP: f32 = KEYS_TOP - FACTS_AIR - FACT_ROWS as f32 * LINE_BLOCK;
const FRAME_HEIGHT: f32 = FACTS_TOP - FACTS_AIR - FRAME_TOP;
const FACT_KEY: f32 = 72.0;
const SHEET_FRAME_HEIGHT: f32 = PREVIEW_HEIGHT - FRAME_TOP - BAND_INSET;
const SHEET_WIDTH: f32 = WINDOW_WIDTH - 2.0 * LINE - 2.0 * FILE_INSET;

impl LauncherView {
    pub(super) fn flow_pane(
        &self,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let kit = qol_gpui::kit::kit();
        let results = div()
            .id("launcher-results")
            .h(px(height))
            .w_full()
            .relative()
            .overflow_hidden()
            .bg(view::bg_color());
        let Some(session) = self.state.flow.as_ref() else {
            return results;
        };
        let entry = &session.entry;
        let rows = &session.rows;
        let total = rows.len();
        let range = self.state.scroll_list.visible_range(total);
        let focused = self.state.list_focused;
        let selected = self.state.scroll_list.selected;
        let shown = rows.get(selected);
        let detail = shown.map_or(Detail::Inline, |row| detail_of(session, row));
        let cards: Vec<Stateful<Div>> = range
            .clone()
            .filter_map(|index| {
                rows.get(index)
                    .map(|row| card(row, entry, focused && index == selected, index, cx))
            })
            .collect();
        let cue = kit.scroll_cue(
            ScrollSource::Window {
                first: range.start,
                shown: range.len(),
                total,
            },
            kit.grounds.pane,
        );
        results
            .on_scroll_wheel(cx.listener(
                |this: &mut Self, event: &ScrollWheelEvent, _window, cx: &mut Context<Self>| {
                    let rows = qol_gpui::scroll_list::wheel_rows(&event.delta, PREVIEW_CARD);
                    let count = this.state.flow_result_count();
                    this.state.scroll_list.wheel_by(rows, count);
                    cx.notify();
                },
            ))
            .flex()
            .gap(px(PANEL_GAP))
            .px(px(FILE_INSET))
            .pt(px(LIST_PAD_Y))
            .pb(px(FILE_INSET))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(PREVIEW_HEIGHT))
                    .relative()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap(px(APP_GAP))
                    .children(cards)
                    .child(cue),
            )
            .child(side_panel(
                shown,
                Shown {
                    entry,
                    detail,
                    status: session.status.as_ref(),
                },
                focused,
                window,
            ))
            .when(session.sheet, |pane| {
                pane.children(shown.map(|row| sheet(row, entry, detail, window)))
            })
    }
}

#[derive(Clone, Copy)]
struct Shown<'a> {
    entry: &'a FlowEntry,
    detail: Detail<'a>,
    status: Option<&'a Badge>,
}

fn detail_of<'a>(session: &'a FlowSession, row: &FlowRow) -> Detail<'a> {
    let (Some(_), Some(id)) = (&session.entry.detail, row.spec.id.as_deref()) else {
        return Detail::Inline;
    };
    session
        .details
        .get(id)
        .map_or(Detail::Loading, Detail::Ready)
}

fn card(
    row: &FlowRow,
    entry: &FlowEntry,
    lit: bool,
    index: usize,
    cx: &mut Context<LauncherView>,
) -> Stateful<Div> {
    let kit = qol_gpui::kit::kit();
    let pane = kit.grounds.pane;
    let ground = if lit { kit.grounds.band } else { pane };
    let glow = if lit { 1.0 } else { 0.0 };
    let spec = &row.spec;
    let title = if spec.masked {
        clip::masked_text()
    } else {
        row.title.clone()
    };
    div()
        .id(("launcher-flow-card", index))
        .flex_none()
        .w_full()
        .h(px(PREVIEW_CARD))
        .px(px(SPACE_PAD))
        .flex()
        .items_center()
        .gap(px(SPACE_CELL))
        .rounded(px(RADIUS_CARD))
        .bg(rgb(view::card_ground(glow)))
        .shadow(view::card_cast(glow))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.state.scroll_list.selected = index;
            this.state.list_focused = true;
            cx.notify();
        }))
        .child(clip::tile(row, entry, CARD_TILE, CARD_GLYPH, ground))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(SPACE_TIGHT))
                .child(
                    div()
                        .flex_none()
                        .h(px(NAME_LINE))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .child(
                            div()
                                .text(if spec.mono {
                                    TextStyle::Code
                                } else {
                                    TextStyle::Name
                                })
                                .text_color(rgb(pane.ink))
                                .line_clamp(1)
                                .child(title),
                        ),
                )
                .when_some(row.subtitle.clone(), |text, summary| {
                    text.child(
                        div()
                            .text(TextStyle::Detail)
                            .text_color(rgb(if lit { ground.soft } else { pane.faint }))
                            .line_clamp(1)
                            .child(summary),
                    )
                }),
        )
        .children(
            spec.tags
                .iter()
                .filter(|tag| tag.tone != Some(Tone::Kind))
                .map(|tag| kit.chip(Chip::Tag(tag.text.clone().into()), ground)),
        )
        .children(spec.badge.as_ref().map(|badge| {
            div()
                .flex_none()
                .text(TextStyle::Detail)
                .text_color(rgb(tone_ink(badge.tone, ground)))
                .child(badge.text.clone())
        }))
        .when(lit, |card| card.child(view::enter_cue(1.0)))
}

fn side_panel(row: Option<&FlowRow>, shown: Shown, lit: bool, window: &mut Window) -> Div {
    let entry = shown.entry;
    let fill = view::panel_fill(if lit { 1.0 } else { 0.0 });
    let panel = div()
        .flex_none()
        .w(px(APP_PANEL_WIDTH))
        .h(px(PREVIEW_HEIGHT))
        .relative()
        .overflow_hidden()
        .rounded(px(RADIUS_CARD))
        .bg(rgb(fill));
    let panel = match row {
        Some(row) => {
            let body = body_of(row, false, shown.detail);
            let counted = body.counted();
            let (frame, over) = clip::frame(
                Frame {
                    body,
                    height: FRAME_HEIGHT,
                    width: APP_PANEL_WIDTH,
                    whole: false,
                },
                window,
            );
            let suffix = match (shown.detail, counted) {
                (Detail::Ready(detail), Some(counted)) => {
                    let text = clip::shown(counted, clip::fits(FRAME_HEIGHT), over);
                    detail.shown_fact.as_deref().map(|key| (key, text))
                }
                _ => None,
            };
            panel
                .child(band(row, entry, None))
                .child(frame.top(px(FRAME_TOP)))
                .child(facts(row, suffix))
        }
        None => panel,
    };
    panel.child(key_line(entry, row.is_some(), shown.status))
}

fn band(row: &FlowRow, entry: &FlowEntry, trailing: Option<Div>) -> Div {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    let (name, sub) = band_text(row);
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(HEAD_HEIGHT))
        .px(px(BAND_INSET))
        .flex()
        .items_center()
        .gap(px(SPACE_CELL))
        .bg(rgba(translucent(0x000000, BAND_SHADE_ALPHA)))
        .child(clip::tile(row, entry, HEAD_TILE, HEAD_GLYPH, ground))
        .child(
            kit.heading_title(TextStyle::Heading, name, None, true)
                .when_some(sub, |text, sub| {
                    text.child(
                        div()
                            .text(TextStyle::Detail)
                            .text_color(rgb(ground.soft))
                            .line_clamp(1)
                            .child(sub),
                    )
                }),
        )
        .children(trailing)
}

fn band_text(row: &FlowRow) -> (String, Option<String>) {
    let spec = &row.spec;
    let name = spec
        .tags
        .iter()
        .find(|tag| tag.tone == Some(Tone::Kind))
        .map(|tag| tag.text.clone())
        .or_else(|| spec.kind.as_deref().map(capitalised))
        .unwrap_or_else(|| row.title.clone());
    let sub = row.subtitle.as_deref().map(|subtitle| {
        subtitle
            .strip_prefix(name.as_str())
            .and_then(|rest| rest.strip_prefix(" \u{b7} "))
            .unwrap_or(subtitle)
            .to_string()
    });
    (name, sub.filter(|sub| !sub.is_empty()))
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn facts(row: &FlowRow, suffix: Option<(&str, String)>) -> Div {
    let ground = qol_gpui::kit::kit().grounds.menu;
    div()
        .absolute()
        .top(px(FACTS_TOP))
        .left(px(BAND_INSET))
        .right(px(BAND_INSET))
        .flex()
        .flex_col()
        .text(TextStyle::Detail)
        .children(row.spec.facts.iter().take(FACT_ROWS).map(|(key, value)| {
            div()
                .flex_none()
                .h(px(LINE_BLOCK))
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_none()
                        .w(px(FACT_KEY))
                        .text_color(rgb(ground.faint))
                        .line_clamp(1)
                        .child(key.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(rgb(ground.ink))
                        .line_clamp(1)
                        .child(fact_value(key, value, suffix.as_ref())),
                )
        }))
}

fn fact_value(key: &str, value: &str, suffix: Option<&(&str, String)>) -> String {
    match suffix {
        Some((shown_key, suffix)) if *shown_key == key => format!("{value}{suffix}"),
        _ => value.to_string(),
    }
}

fn key_line(entry: &FlowEntry, has_row: bool, status: Option<&Badge>) -> Div {
    let kit = qol_gpui::kit::kit();
    let ground = kit.grounds.menu;
    div()
        .absolute()
        .top(px(KEYS_TOP))
        .left(px(BAND_INSET))
        .right(px(BAND_INSET))
        .h(px(LINE_BLOCK))
        .flex()
        .items_center()
        .gap(px(SPACE_PAD))
        .text(TextStyle::Hint)
        .text_color(rgb(ground.soft))
        .when(has_row, |line| {
            line.child(kit.hint(Key::ENTER, enter_label(entry)))
                .child(kit.hint(Key::SPACE, "Preview"))
        })
        .children(status.map(|status| {
            div()
                .ml_auto()
                .child(kit.chip(status_chip(status), kit.grounds.pane))
        }))
}

fn sheet(row: &FlowRow, entry: &FlowEntry, detail: Detail, window: &mut Window) -> Div {
    let kit = qol_gpui::kit::kit();
    let keys = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(SPACE_CELL))
        .text(TextStyle::Hint)
        .text_color(rgb(kit.grounds.menu.soft))
        .child(kit.hint(Key::ENTER, sheet_enter_label(entry)))
        .child(kit.hint(Key::ESC, "close"));
    let (frame, _) = clip::frame(
        Frame {
            body: body_of(row, true, detail),
            height: SHEET_FRAME_HEIGHT,
            width: SHEET_WIDTH,
            whole: true,
        },
        window,
    );
    div()
        .absolute()
        .left(px(FILE_INSET))
        .right(px(FILE_INSET))
        .top(px(LIST_PAD_Y))
        .h(px(PREVIEW_HEIGHT))
        .overflow_hidden()
        .rounded(px(RADIUS_CARD))
        .bg(rgb(view::panel_fill(1.0)))
        .shadow(view::card_cast(0.0))
        .child(band(row, entry, Some(keys)))
        .child(frame.top(px(FRAME_TOP)))
}

fn enter_label(entry: &FlowEntry) -> String {
    match &entry.enter {
        FlowEnter::Copy => "Copy".to_string(),
        FlowEnter::Paste => "Paste".to_string(),
        FlowEnter::Detail => "Open".to_string(),
        FlowEnter::Action(id) => entry
            .row_actions
            .iter()
            .find(|action| &action.action == id)
            .and_then(|action| action.label.clone())
            .unwrap_or_else(|| capitalised(id)),
    }
}

fn sheet_enter_label(entry: &FlowEntry) -> String {
    match &entry.enter {
        FlowEnter::Copy => "copy all".to_string(),
        _ => enter_label(entry).to_lowercase(),
    }
}

fn tone_ink(tone: Option<Tone>, ground: Ground) -> u32 {
    let palette = qol_gpui::kit::kit().palette;
    match tone {
        None => ground.soft,
        Some(Tone::Kind) => palette.accent_ink,
        Some(Tone::Info) => palette.info,
        Some(Tone::Warning) => palette.warning_ink,
        Some(Tone::Error) => palette.danger,
    }
}

fn status_chip(status: &Badge) -> Chip {
    let tone = match status.tone {
        None => StatusTone::Neutral,
        Some(Tone::Info) => StatusTone::Info,
        Some(Tone::Kind) => StatusTone::Accent,
        Some(Tone::Warning) => StatusTone::Warning,
        Some(Tone::Error) => StatusTone::Danger,
    };
    Chip::Status {
        tone,
        text: status.text.clone().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use qol_plugin_api::launcher_flows::{FlowPanel, FlowRow as RowSpec, Tag};

    fn entry(enter: FlowEnter) -> FlowEntry {
        FlowEntry {
            plugin_id: "qol-clipboard".to_string(),
            title: "Clipboard".to_string(),
            prompt: "Search clipboard".to_string(),
            query: "rows".to_string(),
            row_actions: vec![qol_config::contract::RowActionSpec {
                action: "pin".to_string(),
                input: None,
                label: Some("Pin".to_string()),
                description: None,
                key: Some("ctrl+p".to_string()),
                when: None,
            }],
            icon: None,
            enter,
            panel: FlowPanel::Preview,
            detail: None,
        }
    }

    fn row(spec: RowSpec, subtitle: Option<&str>) -> FlowRow {
        FlowRow {
            title: spec.title.clone(),
            subtitle: subtitle.map(str::to_string),
            copy: None,
            raw: serde_json::json!({}),
            spec,
        }
    }

    #[test]
    fn the_side_panel_slots_match_the_locked_board() {
        assert_eq!(FRAME_TOP, 68.0);
        assert_eq!(KEYS_TOP, 498.0);
        assert_eq!(FACTS_TOP, 400.0);
        assert_eq!(FRAME_HEIGHT, 322.0);
        assert_eq!(SHEET_FRAME_HEIGHT, 452.0);
        assert_eq!(HEAD_HEIGHT, 60.0);
        assert_eq!(HEAD_TILE, 44.0);
        assert_eq!(BAND_INSET, 14.0);
    }

    #[test]
    fn the_band_names_the_kind_and_keeps_app_and_age() {
        let tagged = row(
            RowSpec {
                title: "github.com/qol-tools".to_string(),
                kind: Some("link".to_string()),
                tags: vec![Tag {
                    text: "Link".to_string(),
                    tone: Some(Tone::Kind),
                }],
                ..RowSpec::default()
            },
            Some("Link \u{b7} Firefox \u{b7} 2 min"),
        );
        assert_eq!(
            band_text(&tagged),
            ("Link".to_string(), Some("Firefox \u{b7} 2 min".to_string()))
        );
        let kind_only = row(
            RowSpec {
                title: "ssh".to_string(),
                kind: Some("code".to_string()),
                ..RowSpec::default()
            },
            Some("Kitty \u{b7} 3 h"),
        );
        assert_eq!(
            band_text(&kind_only),
            ("Code".to_string(), Some("Kitty \u{b7} 3 h".to_string()))
        );
        let bare = row(
            RowSpec {
                title: "plain".to_string(),
                ..RowSpec::default()
            },
            None,
        );
        assert_eq!(band_text(&bare), ("plain".to_string(), None));
    }

    #[test]
    fn enter_labels_follow_the_manifest() {
        assert_eq!(enter_label(&entry(FlowEnter::Copy)), "Copy");
        assert_eq!(sheet_enter_label(&entry(FlowEnter::Copy)), "copy all");
        assert_eq!(enter_label(&entry(FlowEnter::Paste)), "Paste");
        assert_eq!(
            enter_label(&entry(FlowEnter::Action("pin".to_string()))),
            "Pin"
        );
        assert_eq!(
            enter_label(&entry(FlowEnter::Action("forget".to_string()))),
            "Forget"
        );
        assert_eq!(sheet_enter_label(&entry(FlowEnter::Detail)), "open");
    }

    #[test]
    fn only_the_named_fact_says_how_much_is_shown() {
        let suffix = ("Size", ", all shown".to_string());
        assert_eq!(
            fact_value("Size", "612 characters", Some(&suffix)),
            "612 characters, all shown"
        );
        assert_eq!(fact_value("From", "Firefox", Some(&suffix)), "Firefox");
        assert_eq!(fact_value("Size", "13 KB", None), "13 KB");
    }

    #[test]
    fn a_flow_with_a_detail_query_waits_for_the_shown_rows_detail() {
        let mut state = super::super::state::LauncherState::new();
        state.enter_flow(FlowEntry {
            detail: Some("detail".to_string()),
            ..entry(FlowEnter::Copy)
        });
        let spec = RowSpec {
            title: "l1".to_string(),
            id: Some("c1".to_string()),
            ..RowSpec::default()
        };
        let shown = row(spec.clone(), None);
        let anonymous = row(RowSpec { id: None, ..spec }, None);
        let session = state.flow.as_mut().unwrap();
        assert_eq!(detail_of(session, &shown), Detail::Loading);
        assert_eq!(detail_of(session, &anonymous), Detail::Inline);
        session.details.insert("c1".to_string(), Default::default());
        let session = state.flow.as_ref().unwrap();
        assert!(matches!(detail_of(session, &shown), Detail::Ready(_)));
    }
}
