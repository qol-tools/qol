use std::time::SystemTime;

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, rgb, rgba, uniform_list, AnyElement, Context, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, Styled,
};
use qol_gpui::kit::{kit, Chip, Kit};
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{
    TextStyle, HEIGHT_CONTROL, HEIGHT_INLINE, LINE, SPACE_CELL, SPACE_GUTTER, SPACE_INSET,
    SPACE_PAD, SPACE_TIGHT,
};
use qol_gpui::Key;

use super::choices::{choice_index, newer_here, newer_incoming, say, stamp_label, CHOICES};
use super::pictogram::{named_pictogram, PictogramSize};
use super::{ExplorerView, BAND};
use crate::ops::{Clash, Room, Survey};

const STAMP_WIDTH: f32 = 150.0;

pub(super) fn render(
    view: &ExplorerView,
    survey: &Survey,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let kit = kit();
    let pane = kit.grounds.pane;
    let blocked = matches!(survey.room, Room::Short { .. });
    let clashes = survey.clashes.len();
    let newer = survey
        .clashes
        .iter()
        .filter(|clash| newer_here(clash))
        .count();
    let from = survey
        .transfer
        .sources
        .first()
        .and_then(|source| source.parent())
        .and_then(|parent| parent.file_name())
        .map_or_else(String::new, crate::listing::display_name);
    let lines = say(view, survey);
    let apply = if clashes > 0 {
        "Apply choices"
    } else {
        "Continue"
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
                .flex()
                .flex_col()
                .gap(px(SPACE_TIGHT))
                .px(px(SPACE_GUTTER))
                .py(px(SPACE_PAD))
                .children(lines.into_iter().map(|line| {
                    div()
                        .text(TextStyle::Name)
                        .text_color(rgb(pane.ink))
                        .child(line)
                })),
        )
        .when(clashes > 0, |sheet| {
            sheet
                .child(columns(kit, &from))
                .child(table(view, survey, kit, cx))
                .when(newer > 0, |sheet| sheet.child(hint(kit, newer)))
        })
        .when(clashes == 0, |sheet| sheet.child(div().flex_1()))
        .child(buttons(kit, apply, blocked, cx))
        .into_any_element()
}

fn label(text: String, kit: Kit) -> gpui::Div {
    div()
        .text(TextStyle::Label)
        .text_color(rgb(kit.grounds.pane.faint))
        .child(text)
}

fn columns(kit: Kit, from: &str) -> AnyElement {
    div()
        .flex_none()
        .h(px(HEIGHT_INLINE))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_CELL))
        .px(px(SPACE_GUTTER))
        .border_b(px(LINE))
        .border_color(rgba(kit.washes.hairline.packed()))
        .child(label("File".to_string(), kit).flex_1().min_w_0())
        .child(
            label("Here".to_string(), kit)
                .flex_none()
                .w(px(STAMP_WIDTH)),
        )
        .child(
            label(format!("From {from}"), kit)
                .flex_none()
                .w(px(STAMP_WIDTH))
                .overflow_hidden(),
        )
        .child(div().flex_none().w(px(segment_width())))
        .into_any_element()
}

fn segment_width() -> f32 {
    260.0
}

fn table(
    view: &ExplorerView,
    survey: &Survey,
    kit: Kit,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let clashes = survey.clashes.clone();
    let selected = view.clash_row;
    let count = clashes.len();
    div()
        .flex_1()
        .min_h_0()
        .child(
            uniform_list(
                "explorer-clashes",
                count,
                cx.processor(move |_, rows: std::ops::Range<usize>, _window, cx| {
                    let now = SystemTime::now();
                    rows.filter_map(|row| {
                        let clash = clashes.get(row)?;
                        Some(clash_row(clash, row, row == selected, now, kit, cx))
                    })
                    .collect::<Vec<_>>()
                }),
            )
            .size_full()
            .track_scroll(view.clash_scroll.clone()),
        )
        .into_any_element()
}

fn clash_row(
    clash: &Clash,
    row: usize,
    selected: bool,
    now: SystemTime,
    kit: Kit,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let ground = kit.highlight_ground(selected);
    let name = clash
        .existing
        .file_name()
        .map_or_else(String::new, crate::listing::display_name);
    let segments = CHOICES.iter().map(|(choice, text)| {
        let choice = *choice;
        kit.segment(*text, clash.choice == choice)
            .id((
                "explorer-clash-choice",
                row * CHOICES.len() + choice_index(choice),
            ))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, _window, cx| {
                view.choose_clash(row, choice, cx);
            }))
    });
    let element = div()
        .id(("explorer-clash", row))
        .w_full()
        .h(px(HEIGHT_CONTROL + SPACE_INSET))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_CELL))
        .px(px(SPACE_GUTTER))
        .child(div().flex_none().w(px(16.0)).child(named_pictogram(
            &name,
            clash.here.folder,
            false,
            PictogramSize::Small,
            selected,
        )))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text(TextStyle::ListName)
                .text_color(rgb(ground.ink))
                .child(name),
        )
        .child(stamp_cell(
            stamp_label(&clash.here, newer_here(clash), now),
            ground.soft,
        ))
        .child(stamp_cell(
            stamp_label(&clash.incoming, newer_incoming(clash), now),
            ground.soft,
        ))
        .child(
            kit.segmented_group()
                .w(px(segment_width()))
                .justify_end()
                .children(segments),
        )
        .on_click(cx.listener(move |view, _, _window, cx| {
            view.select_clash(row, cx);
        }));
    kit.highlight(element, selected).into_any_element()
}

fn stamp_cell(text: String, ink: u32) -> gpui::Div {
    div()
        .flex_none()
        .w(px(STAMP_WIDTH))
        .overflow_hidden()
        .whitespace_nowrap()
        .text(TextStyle::Detail)
        .text_color(rgb(ink))
        .child(text)
}

fn hint(kit: Kit, newer: usize) -> AnyElement {
    let pane = kit.grounds.pane;
    let key = |key: Key| kit.chip(Chip::Key(key), pane);
    div()
        .flex_none()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(SPACE_TIGHT))
        .px(px(SPACE_GUTTER))
        .py(px(SPACE_CELL))
        .text(TextStyle::Hint)
        .text_color(rgb(pane.faint))
        .child(format!(
            "The {newer} here {} newer, so {} kept unless you choose.",
            if newer == 1 { "is" } else { "are" },
            if newer == 1 { "it is" } else { "they are" },
        ))
        .child(key(Key::LEFT))
        .child("or")
        .child(key(Key::RIGHT))
        .child("chooses, and with")
        .child(kit.chip(Chip::KeyText("shift".into()), pane))
        .child("it chooses for every row.")
        .into_any_element()
}

fn buttons(
    kit: Kit,
    apply: &'static str,
    blocked: bool,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let cancel = kit
        .button_ghost("Cancel")
        .id("explorer-clash-cancel")
        .cursor_pointer()
        .on_click(cx.listener(|view, _, _window, cx| view.cancel_clash(cx)));
    let go = kit
        .button_inked(kit.grounds.pane)
        .id("explorer-clash-apply")
        .when(!blocked, |go| {
            go.cursor_pointer()
                .on_click(cx.listener(|view, _, _window, cx| view.apply_clash(cx)))
        })
        .when(blocked, |go| go.opacity(qol_gpui::theme::OPACITY_DISABLED))
        .child(apply);
    div()
        .flex_none()
        .flex()
        .flex_row()
        .justify_end()
        .gap(px(SPACE_CELL))
        .px(px(SPACE_GUTTER))
        .pb(px(SPACE_PAD))
        .child(cancel)
        .child(go)
        .into_any_element()
}
