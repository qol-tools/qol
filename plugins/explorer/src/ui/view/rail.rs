use std::path::PathBuf;

use gpui::{
    div, px, rgb, AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled,
};
use qol_gpui::kit::{kit, Kit};
use qol_gpui::scrollbar::{seek_handle, ScrollGrip, ScrollSource};
use qol_gpui::settings_panel::components::rail_caption;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{TextStyle, HEIGHT_CONTROL, SPACE_GUTTER, SPACE_INSET};

use super::pictogram::{rail_pictogram, RailGlyph};
use super::{ExplorerView, RAIL};
use crate::places::PlaceKind;
use crate::ui::trail;

struct RailRow {
    glyph: RailGlyph,
    label: String,
    path: PathBuf,
}

pub(super) fn render(view: &ExplorerView, cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let kit = kit();
    let places: Vec<RailRow> = view
        .places
        .iter()
        .map(|place| RailRow {
            glyph: place_glyph(place.kind),
            label: place.kind.label().to_string(),
            path: place.path.clone(),
        })
        .collect();
    let devices: Vec<RailRow> = view
        .devices
        .iter()
        .map(|device| RailRow {
            glyph: RailGlyph::Drive,
            label: device.label.clone(),
            path: device.path.clone(),
        })
        .collect();
    let current = trail::deepest(
        view.browser.folder(),
        places.iter().chain(&devices).map(|row| row.path.as_path()),
    );
    let place_count = places.len();
    let in_devices = current.is_some_and(|index| index >= place_count);
    let mut children: Vec<AnyElement> = Vec::new();
    children.push(header("Places", "Your folders", !in_devices));
    for (index, row) in places.into_iter().enumerate() {
        children.push(render_row(index, row, current == Some(index), kit, cx));
    }
    children.push(header("Devices", "Your drives", in_devices));
    for (offset, row) in devices.into_iter().enumerate() {
        let index = place_count + offset;
        children.push(render_row(index, row, current == Some(index), kit, cx));
    }
    let source = ScrollSource::Handle {
        handle: view.rail_scroll.clone(),
        children: children.len(),
    };
    div()
        .flex_none()
        .w(px(RAIL))
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

fn place_glyph(kind: PlaceKind) -> RailGlyph {
    match kind {
        PlaceKind::Home => RailGlyph::Home,
        PlaceKind::Desktop
        | PlaceKind::Documents
        | PlaceKind::Downloads
        | PlaceKind::Music
        | PlaceKind::Pictures
        | PlaceKind::Videos => RailGlyph::Folder,
    }
}

fn header(label: &'static str, colophon: &'static str, live: bool) -> AnyElement {
    div()
        .flex_none()
        .px(px(SPACE_INSET))
        .child(rail_caption(label, Some(colophon.into()), live))
        .into_any_element()
}

fn render_row(
    index: usize,
    row: RailRow,
    current: bool,
    kit: Kit,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let ink = if current {
        kit.grounds.band.ink
    } else {
        kit.grounds.rail.soft
    };
    let path = row.path;
    let element = div()
        .id(("explorer-rail-row", index))
        .flex_none()
        .h(px(HEIGHT_CONTROL))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .child(rail_pictogram(row.glyph, current))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text(TextStyle::Name)
                .text_color(rgb(ink))
                .child(row.label),
        )
        .on_click(cx.listener(move |view, _: &ClickEvent, _window, cx| {
            view.navigate(path.clone(), cx);
        }));
    if current {
        return kit.highlight(element, true).into_any_element();
    }
    kit.pointable(element, rgb(kit.grounds.rail.lift))
        .into_any_element()
}
