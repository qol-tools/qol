use std::ops::Range;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::{
    div, px, rgb, rgba, uniform_list, AnyElement, ClickEvent, Context, Div, InteractiveElement,
    IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window,
};
use qol_gpui::icon::icon;
use qol_gpui::kit::{kit, Kit};
use qol_gpui::scrollbar::{ScrollGrip, ScrollSource};
use qol_gpui::text::{baseline, cased, line_height, TextStyled};
use qol_gpui::theme::{
    translucent, Alpha, Ground, TextStyle, HEIGHT_INLINE, LINE, SPACE_GUTTER, SPACE_INSET,
};
use qol_gpui::Icon;

use super::{ExplorerView, HEAD, ICON, ROW};
use crate::listing::ListingState;
use crate::ui::columns::{kind_label, left_to_right, size_label, typed_split};

const KIND_WIDTH: f32 = 100.0;
const SIZE_WIDTH: f32 = 100.0;

pub(super) fn render(view: &ExplorerView, cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let kit = kit();
    let listing = view.browser.listing();
    let loading = *listing.state() == ListingState::Loading;
    let body = match listing.state() {
        ListingState::Failed(error) => kit.empty(error.to_string(), None).into_any_element(),
        ListingState::Done if listing.shows_empty(view.browser.shows_hidden()) => {
            kit.empty("This folder is empty", None).into_any_element()
        }
        ListingState::Done | ListingState::Loading => rows(view, kit, cx),
    };
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(rgb(kit.grounds.pane.bg))
        .child(header(kit))
        .child(body)
        .when(loading, |list| list.child(reading_line(view, kit)))
}

fn header(kit: Kit) -> impl IntoElement {
    let faint = kit.grounds.pane.faint;
    let label = |text: &str| {
        div()
            .text(TextStyle::Label)
            .text_color(rgb(faint))
            .child(cased(TextStyle::Label, text))
    };
    div()
        .flex_none()
        .h(px(HEAD))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .border_b(px(LINE))
        .border_color(rgba(kit.washes.hairline.packed()))
        .child(div().flex_none().w(px(ICON)))
        .child(label("Name").flex_1().min_w_0())
        .child(label("Kind").flex_none().w(px(KIND_WIDTH)))
        .child(
            div()
                .flex_none()
                .w(px(SIZE_WIDTH))
                .flex()
                .justify_end()
                .child(label("Size")),
        )
}

fn rows(view: &ExplorerView, kit: Kit, cx: &mut Context<ExplorerView>) -> AnyElement {
    let total = view.browser.row_count();
    let scroll = view.scroll_offset();
    let first = ((scroll - 0.5) / ROW).ceil().max(0.0) as usize;
    let end = ((scroll + view.list_height() + 0.5) / ROW).floor().max(0.0) as usize;
    let shown = end.min(total).saturating_sub(first);
    let source = ScrollSource::Window {
        first,
        shown,
        total,
    };
    let seek = cx.entity().downgrade();
    div()
        .flex_1()
        .min_h_0()
        .relative()
        .child(
            uniform_list(
                "explorer-rows",
                total,
                cx.processor(|view, range: Range<usize>, window, cx| {
                    view.browser.show_rows(range.clone());
                    let now = Instant::now();
                    range
                        .filter_map(|row| render_row(view, row, now, window, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .size_full()
            .track_scroll(view.scroll.clone()),
        )
        .child(kit.scroll_cue(source.clone(), kit.grounds.pane))
        .child(ScrollGrip::new(
            "explorer-rows-grip",
            source,
            move |at, _window, cx| {
                let _ = seek.update(cx, |view, cx| view.seek_rows(at, cx));
            },
        ))
        .into_any_element()
}

fn render_row(
    view: &ExplorerView,
    row: usize,
    now: Instant,
    window: &mut Window,
    cx: &mut Context<ExplorerView>,
) -> Option<AnyElement> {
    let kit = kit();
    let browser = &view.browser;
    let entry = browser.entry(row)?;
    let path = entry.path.clone();
    let picked = browser.is_picked(entry);
    let cursor = row == browser.cursor();
    let ground = if picked {
        kit.grounds.band
    } else {
        kit.grounds.pane
    };
    let typed = cursor
        .then(|| typed_split(&entry.display, browser.jump_run(now)))
        .flatten();
    let size = size_label(
        entry,
        browser.listing().item_count(&entry.path),
        browser.shows_hidden(),
    );
    let element = div()
        .id(("explorer-row", row))
        .relative()
        .w_full()
        .h(px(ROW))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .when(cursor, |element| {
            element.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgba(translucent(ground.ink, Alpha::Tint))),
            )
        })
        .child(div().flex_none().w(px(ICON)).child(icon(
            if entry.opens_as_folder {
                Icon::Folder
            } else {
                Icon::File
            },
            ICON,
            ground.soft,
        )))
        .child(name_cell(&entry.display, typed, ground, window))
        .child(
            div()
                .flex_none()
                .w(px(KIND_WIDTH))
                .text(TextStyle::Detail)
                .text_color(rgb(ground.soft))
                .child(kind_label(entry.kind)),
        )
        .child(
            div()
                .flex_none()
                .w(px(SIZE_WIDTH))
                .flex()
                .justify_end()
                .text(TextStyle::Detail)
                .text_color(rgb(ground.soft))
                .child(size),
        )
        .on_click(cx.listener(move |view, event: &ClickEvent, _window, cx| {
            view.click_row(&path, event, cx);
        }));
    Some(kit.highlight(element, picked).into_any_element())
}

fn name_cell(name: &str, typed: Option<usize>, ground: Ground, window: &mut Window) -> Div {
    let cell = div().flex_1().min_w_0().flex().flex_row();
    let Some(split) = typed else {
        return cell.child(name_text(name, ground.ink));
    };
    let (prefix, rest) = name.split_at(split);
    let rise = baseline(window, TextStyle::Title) - baseline(window, TextStyle::ListName);
    cell.h(px(line_height(TextStyle::ListName)))
        .items_start()
        .child(
            div()
                .flex_none()
                .relative()
                .top(px(-rise))
                .text(TextStyle::Title)
                .text_color(rgb(ground.mark))
                .child(left_to_right(prefix)),
        )
        .child(name_text(rest, ground.ink))
}

fn name_text(name: &str, ink: u32) -> Div {
    div()
        .min_w_0()
        .text(TextStyle::ListName)
        .text_color(rgb(ink))
        .child(left_to_right(name))
}

fn reading_line(view: &ExplorerView, kit: Kit) -> impl IntoElement {
    let folder = view.browser.folder();
    let name = folder.file_name().map_or_else(
        || folder.display().to_string(),
        crate::listing::display_name,
    );
    div()
        .flex_none()
        .h(px(HEIGHT_INLINE))
        .flex()
        .items_center()
        .pl(px(SPACE_GUTTER + ICON + SPACE_INSET))
        .pr(px(SPACE_GUTTER))
        .text(TextStyle::Detail)
        .text_color(rgb(kit.grounds.pane.faint))
        .child(format!("Reading {name}"))
}
