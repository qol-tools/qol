use std::path::PathBuf;

use gpui::{
    div, px, rgb, rgba, AnyElement, ClickEvent, Context, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, Styled,
};
use qol_gpui::icon::icon;
use qol_gpui::kit::{kit, Kit};
use qol_gpui::scrollbar::ScrollSource;
use qol_gpui::settings_panel::components::{paint_rail_selection, rail_caption};
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{TextStyle, LINE, SPACE_GUTTER, SPACE_INSET};
use qol_gpui::Icon;

use super::{ExplorerView, ICON, RAIL, ROW};
use crate::places::PlaceKind;
use crate::ui::trail;

struct RailRow {
    icon: Icon,
    label: String,
    path: PathBuf,
}

pub(super) fn render(view: &ExplorerView, cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let kit = kit();
    let places: Vec<RailRow> = view
        .places
        .iter()
        .map(|place| RailRow {
            icon: place_icon(place.kind),
            label: place.kind.label().to_string(),
            path: place.path.clone(),
        })
        .collect();
    let devices: Vec<RailRow> = view
        .devices
        .iter()
        .map(|device| RailRow {
            icon: Icon::Drive,
            label: device.label.clone(),
            path: device.path.clone(),
        })
        .collect();
    let current = trail::deepest(
        view.browser.folder(),
        places.iter().chain(&devices).map(|row| row.path.as_path()),
    );
    let place_count = places.len();
    let in_places = current.is_some_and(|index| index < place_count);
    let in_devices = current.is_some_and(|index| index >= place_count);
    let mut children: Vec<AnyElement> = Vec::new();
    children.push(header("Places", "Your folders", in_places));
    for (index, row) in places.into_iter().enumerate() {
        children.push(render_row(index, row, current == Some(index), kit, cx));
    }
    children.push(header("Devices", "Your drives", in_devices));
    for (offset, row) in devices.into_iter().enumerate() {
        let index = place_count + offset;
        children.push(render_row(index, row, current == Some(index), kit, cx));
    }
    let count = children.len();
    div()
        .flex_none()
        .w(px(RAIL))
        .h_full()
        .relative()
        .bg(rgb(kit.grounds.pane.bg))
        .border_r(px(LINE))
        .border_color(rgba(kit.washes.hairline.packed()))
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
        .child(kit.scroll_cue(
            ScrollSource::Handle {
                handle: view.rail_scroll.clone(),
                children: count,
            },
            kit.grounds.pane,
        ))
}

fn place_icon(kind: PlaceKind) -> Icon {
    match kind {
        PlaceKind::Home => Icon::Home,
        PlaceKind::Desktop
        | PlaceKind::Documents
        | PlaceKind::Downloads
        | PlaceKind::Music
        | PlaceKind::Pictures
        | PlaceKind::Videos => Icon::Folder,
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
    let pane = kit.grounds.pane;
    let ink = if current { pane.ink } else { pane.soft };
    let path = row.path;
    let element = div()
        .id(("explorer-rail-row", index))
        .flex_none()
        .h(px(ROW))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .child(icon(row.icon, ICON, ink))
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
        return paint_rail_selection(element, kit, false).into_any_element();
    }
    kit.pointable(element, rgb(pane.lift)).into_any_element()
}
