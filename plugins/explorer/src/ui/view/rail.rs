use std::path::PathBuf;

use gpui::{
    div, px, rgb, rgba, AnimationExt as _, AnyElement, ClickEvent, Context, Div,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    StatefulInteractiveElement, Styled,
};
use qol_gpui::kit::{float_shadow, kit, Kit};
use qol_gpui::motion;
use qol_gpui::scrollbar::{seek_handle, ScrollGrip, ScrollSource};
use qol_gpui::settings_panel::components::rail_caption;
use qol_gpui::text::TextStyled;
use qol_gpui::text_edit::TextField;
use qol_gpui::theme::{
    Motion, TextStyle, HEIGHT_CONTROL, HEIGHT_INLINE, OPACITY_DISABLED, SPACE_CELL, SPACE_GUTTER,
    SPACE_INSET, SPACE_TIGHT,
};

use super::pictogram::{rail_pictogram, RailGlyph};
use super::rename::RenameLabel;
use super::targets::{self, Zone};
use super::{more_button, ExplorerView};
use crate::ui::trail;

const GLYPH_WIDTH: f32 = 16.0;

struct RailRow {
    glyph: RailGlyph,
    label: String,
    path: PathBuf,
    place: Option<usize>,
    connected: bool,
    renaming: Option<TextField>,
}

pub(super) fn render(view: &ExplorerView, cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let kit = kit();
    let places: Vec<RailRow> = view
        .places
        .iter()
        .enumerate()
        .map(|(index, place)| RailRow {
            glyph: if place.home {
                RailGlyph::Home
            } else {
                RailGlyph::Folder
            },
            label: place.name.clone(),
            path: place.path.clone(),
            place: Some(index),
            connected: place.connected,
            renaming: view
                .renaming
                .as_ref()
                .filter(|renaming| renaming.pin == place.pin)
                .map(|renaming| renaming.field.clone()),
        })
        .collect();
    let devices: Vec<RailRow> = view
        .devices
        .iter()
        .map(|device| RailRow {
            glyph: RailGlyph::Drive,
            label: device.label.clone(),
            path: device.path.clone(),
            place: None,
            connected: true,
            renaming: None,
        })
        .collect();
    let current = trail::deepest(
        view.browser.folder(),
        places.iter().chain(&devices).map(|row| row.path.as_path()),
    );
    let place_count = places.len();
    let in_devices = current.is_some_and(|index| index >= place_count);
    let mut children: Vec<AnyElement> = Vec::new();
    let width = view.rail_width;
    children.push(header("Pinned", "Your folders", !in_devices));
    let landing = view.pin_landing();
    let mut pinned: Vec<AnyElement> = Vec::new();
    for (index, row) in places.into_iter().enumerate() {
        if let Some((slot, name)) = landing.as_ref().filter(|(slot, _)| *slot == index) {
            pinned.push(pin_slot(*slot, name.clone(), kit));
        }
        let tag = view.drop_overlay(Zone::Rail(index), kit.grounds.rail);
        pinned.push(render_row(
            index,
            row,
            current == Some(index),
            width,
            tag,
            kit,
            cx,
        ));
    }
    if let Some((slot, name)) = landing.filter(|(slot, _)| *slot >= place_count) {
        pinned.push(pin_slot(slot, name, kit));
    }
    children.push(
        targets::pins(
            div()
                .flex_none()
                .min_h(px(HEIGHT_CONTROL))
                .flex()
                .flex_col(),
            cx,
        )
        .children(pinned)
        .into_any_element(),
    );
    children.push(header("Devices", "Your drives", in_devices));
    for (offset, row) in devices.into_iter().enumerate() {
        let index = place_count + offset;
        let tag = view.drop_overlay(Zone::Rail(index), kit.grounds.rail);
        children.push(render_row(
            index,
            row,
            current == Some(index),
            width,
            tag,
            kit,
            cx,
        ));
    }
    let source = ScrollSource::Handle {
        handle: view.rail_scroll.clone(),
        children: children.len(),
    };
    div()
        .flex_none()
        .w(px(view.rail_width))
        .h_full()
        .relative()
        .bg(rgb(kit.grounds.rail.bg))
        .child(
            div()
                .id("explorer-rail")
                .size_full()
                .flex()
                .flex_col()
                .pt(px(SPACE_INSET))
                .overflow_y_scroll()
                .track_scroll(&view.rail_scroll)
                .children(children),
        )
        .child(kit.scroll_cue(source.clone(), kit.grounds.rail))
        .child(ScrollGrip::new(
            "explorer-rail-grip",
            source,
            seek_handle(view.rail_scroll.clone()),
        ))
}

fn header(label: &'static str, colophon: &'static str, live: bool) -> AnyElement {
    div()
        .flex_none()
        .px(px(SPACE_INSET))
        .child(rail_caption(label, Some(colophon.into()), live))
        .into_any_element()
}

fn pin_slot(slot: usize, name: String, kit: Kit) -> AnyElement {
    let pane = kit.grounds.pane;
    let row = div()
        .absolute()
        .top(px(-SPACE_INSET))
        .left(px(SPACE_CELL))
        .right(px(-SPACE_TIGHT))
        .h(px(HEIGHT_CONTROL))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .bg(rgb(pane.bg))
        .shadow(float_shadow(pane.ink))
        .child(rail_pictogram(RailGlyph::Folder, false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text(TextStyle::Name)
                .text_color(rgb(pane.ink))
                .child(name),
        );
    div()
        .flex_none()
        .relative()
        .bg(rgba(kit.grounds.rail.well.packed()))
        .child(row)
        .with_animation(
            ("explorer-pin-slot", slot),
            motion::animation(Motion::SETTLE),
            |slot, delta| slot.h(px(HEIGHT_CONTROL * delta)),
        )
        .into_any_element()
}

fn render_row(
    index: usize,
    row: RailRow,
    current: bool,
    rail_width: f32,
    tag: Option<Div>,
    kit: Kit,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let ground = if current {
        kit.grounds.band
    } else {
        kit.grounds.rail
    };
    let ink = if current { ground.ink } else { ground.soft };
    let path = row.path;
    let place = row.place;
    let more = place.filter(|_| current && tag.is_none());
    let renaming = row.renaming.is_some();
    let label = match row.renaming {
        Some(field) => {
            let taken = 2.0 * SPACE_GUTTER
                + GLYPH_WIDTH
                + SPACE_INSET
                + if more.is_some() {
                    HEIGHT_INLINE + SPACE_INSET
                } else {
                    0.0
                };
            RenameLabel::on_ground(field, rail_width - taken, TextStyle::Name, ground)
                .into_any_element()
        }
        None => row.label.into_any_element(),
    };
    let mut name = div()
        .flex_1()
        .min_w_0()
        .text(TextStyle::Name)
        .text_color(rgb(ink))
        .child(label);
    if renaming {
        name = name.on_mouse_down_out(cx.listener(|view, _: &MouseDownEvent, _window, cx| {
            view.finish_renaming(cx);
            cx.notify();
        }));
    }
    let mut element = div()
        .id(("explorer-rail-row", index))
        .flex_none()
        .h(px(HEIGHT_CONTROL))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .child(rail_pictogram(row.glyph, current))
        .child(name);
    if !row.connected {
        element = element.opacity(OPACITY_DISABLED).child(
            div()
                .flex_none()
                .text(TextStyle::Detail)
                .text_color(rgb(ground.faint))
                .child("not connected"),
        );
    }
    if let Some(place) = place {
        element = element.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |view, event: &MouseDownEvent, _window, cx| {
                view.open_pin_menu(place, event.position, cx);
            }),
        );
    }
    if let Some(place) = more {
        element = element.child(more_button(
            ("explorer-rail-more", place),
            kit.grounds.band,
            move |view, event, cx| view.toggle_pin_menu(place, event.position, cx),
            cx,
        ));
    }
    if let Some(tag) = tag {
        element = element.relative().bg(rgb(kit.grounds.band.bg)).child(tag);
    }
    let element = if row.connected && place.is_none() {
        targets::target(element, Zone::Rail(index), path.clone(), cx)
    } else {
        element
    };
    let element = element.on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
        view.navigate(path.clone(), cx);
    }));
    if current {
        return kit.highlight(element, true).into_any_element();
    }
    kit.pointable(element, rgb(kit.grounds.rail.lift))
        .into_any_element()
}
