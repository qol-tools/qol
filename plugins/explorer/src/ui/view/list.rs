use std::ops::Range;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, px, rgb, rgba, uniform_list, AnyElement, ClickEvent, Context, DispatchPhase, Div,
    InteractiveElement, IntoElement, ParentElement, ScrollWheelEvent, StatefulInteractiveElement,
    Styled, Window,
};
use qol_gpui::kit::{kit, Kit};
use qol_gpui::scrollbar::{ScrollGrip, ScrollSource};
use qol_gpui::text::{baseline, line_height, TextStyled};
use qol_gpui::theme::{
    translucent, Alpha, Ground, TextStyle, HEIGHT_INLINE, LINE, SPACE_CELL, SPACE_GUTTER,
    SPACE_INSET, SPACE_STACK,
};

use super::grid::{Layout, ACROSS};
use super::pictogram::entry_pictogram;
use super::{ExplorerView, HEAD, ICON};
use crate::listing::{Entry, ListingState};
use crate::ui::columns::{kind_label, left_to_right, size_label, typed_split};

const KIND_WIDTH: f32 = 110.0;
const SIZE_WIDTH: f32 = 90.0;
const NAME_MAX_WIDTH: f32 = 640.0;
const SECOND_LINE_JOIN: &str = "  ·  ";

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
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .bg(rgb(kit.grounds.pane.bg))
        .when(view.stop.shows_columns(), |list| list.child(header(kit)))
        .child(body)
        .when(loading, |list| list.child(reading_line(view, kit)))
        .child(wheel_resize(cx))
}

fn header(kit: Kit) -> impl IntoElement {
    let faint = kit.grounds.pane.faint;
    let label = |text: &'static str| {
        div()
            .text(TextStyle::Label)
            .text_color(rgb(faint))
            .child(text)
    };
    div()
        .flex_none()
        .h(px(HEAD))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_CELL))
        .px(px(SPACE_GUTTER))
        .border_b(px(LINE))
        .border_color(rgba(kit.washes.hairline.packed()))
        .child(div().flex_none().w(px(ICON)))
        .child(label("name").flex_1().min_w_0().max_w(px(NAME_MAX_WIDTH)))
        .child(label("kind").flex_none().w(px(KIND_WIDTH)))
        .child(
            div()
                .flex_none()
                .w(px(SIZE_WIDTH))
                .flex()
                .justify_end()
                .child(label("size")),
        )
}

fn wheel_resize(cx: &mut Context<ExplorerView>) -> impl IntoElement {
    let view = cx.weak_entity();
    canvas(
        |_, _, _| {},
        move |area, _, window, _| {
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _window, cx| {
                if phase != DispatchPhase::Capture
                    || !event.modifiers.secondary()
                    || !area.contains(&event.position)
                {
                    return;
                }
                cx.stop_propagation();
                let _ = view.update(cx, |view, cx| view.wheel_resize(event.delta, cx));
            });
        },
    )
    .absolute()
    .inset_0()
}

fn rows(view: &ExplorerView, kit: Kit, cx: &mut Context<ExplorerView>) -> AnyElement {
    let layout = view.layout();
    let total = layout.lines(view.browser.row_count());
    let scroll = view.scroll_offset();
    let first = ((scroll - 0.5) / layout.pitch).ceil().max(0.0) as usize;
    let end = ((scroll + view.list_height() + 0.5) / layout.pitch)
        .floor()
        .max(0.0) as usize;
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
                cx.processor(|view, lines: Range<usize>, window, cx| {
                    let layout = view.layout();
                    let entries = layout.entries(lines.clone(), view.browser.row_count());
                    view.browser.show_rows(entries);
                    let now = Instant::now();
                    if layout.stop.is_grid() {
                        return lines
                            .map(|line| render_line(view, line, layout, now, window, cx))
                            .collect::<Vec<_>>();
                    }
                    lines
                        .filter_map(|row| render_row(view, row, layout, now, window, cx))
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

struct Shown<'a> {
    entry: &'a Entry,
    picked: bool,
    cursor: bool,
    ground: Ground,
    typed: Option<usize>,
    size: String,
}

fn shown(view: &ExplorerView, row: usize, now: Instant) -> Option<Shown<'_>> {
    let browser = &view.browser;
    let entry = browser.entry(row)?;
    let picked = browser.is_picked(entry);
    let cursor = row == browser.cursor();
    let typed = cursor
        .then(|| typed_split(&entry.display, browser.jump_run(now)))
        .flatten();
    let size = size_label(
        entry,
        browser.listing().item_count(&entry.path),
        browser.shows_hidden(),
    );
    Some(Shown {
        entry,
        picked,
        cursor,
        ground: kit().highlight_ground(picked),
        typed,
        size,
    })
}

fn render_row(
    view: &ExplorerView,
    row: usize,
    layout: Layout,
    now: Instant,
    window: &mut Window,
    cx: &mut Context<ExplorerView>,
) -> Option<AnyElement> {
    let kit = kit();
    let stop = layout.stop;
    let shown = shown(view, row, now)?;
    let entry = shown.entry;
    let ground = shown.ground;
    let path = entry.path.clone();
    let name = name_cell(
        &entry.display,
        shown.typed,
        ground,
        stop.name_style(),
        false,
        window,
    )
    .when(stop.shows_columns(), |name| name.max_w(px(NAME_MAX_WIDTH)));
    let element = div()
        .id(("explorer-row", row))
        .relative()
        .w_full()
        .h(px(layout.pitch))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_CELL))
        .px(px(SPACE_GUTTER))
        .when(shown.cursor, |element| element.child(cursor_wash(ground)))
        .child(div().flex_none().w(px(ICON)).child(entry_pictogram(
            entry,
            stop.pictogram(),
            shown.picked,
        )))
        .map(|element| {
            if !stop.two_lines() {
                return element.child(name);
            }
            element.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(SPACE_STACK))
                    .child(div().flex().flex_row().child(name))
                    .child(detail_text(second_line(entry, &shown.size), ground)),
            )
        })
        .when(stop.shows_columns(), |element| {
            element
                .child(
                    detail_text(kind_text(entry).to_string(), ground)
                        .flex_none()
                        .w(px(KIND_WIDTH)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(SIZE_WIDTH))
                        .flex()
                        .justify_end()
                        .child(detail_text(shown.size, ground)),
                )
        })
        .on_click(cx.listener(move |view, event: &ClickEvent, _window, cx| {
            view.click_row(&path, event, cx);
        }));
    Some(kit.highlight(element, shown.picked).into_any_element())
}

fn render_line(
    view: &ExplorerView,
    line: usize,
    layout: Layout,
    now: Instant,
    window: &mut Window,
    cx: &mut Context<ExplorerView>,
) -> AnyElement {
    let entries = layout.entries(line..line + 1, view.browser.row_count());
    div()
        .id(("explorer-line", line))
        .w_full()
        .h(px(layout.pitch))
        .flex()
        .flex_row()
        .pt(px(SPACE_INSET))
        .px(px(SPACE_GUTTER))
        .gap(px(ACROSS))
        .children(entries.filter_map(|row| render_cell(view, row, layout, now, window, cx)))
        .into_any_element()
}

fn render_cell(
    view: &ExplorerView,
    row: usize,
    layout: Layout,
    now: Instant,
    window: &mut Window,
    cx: &mut Context<ExplorerView>,
) -> Option<AnyElement> {
    let kit = kit();
    let stop = layout.stop;
    let (width, height) = stop.cell()?;
    let shown = shown(view, row, now)?;
    let entry = shown.entry;
    let ground = shown.ground;
    let path = entry.path.clone();
    let name = name_cell(
        &entry.display,
        shown.typed,
        ground,
        stop.name_style(),
        true,
        window,
    );
    let element = div()
        .id(("explorer-cell", row))
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(SPACE_INSET))
        .p(px(SPACE_INSET))
        .when(shown.cursor, |element| element.child(cursor_wash(ground)))
        .child(entry_pictogram(entry, stop.pictogram(), shown.picked))
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(SPACE_STACK))
                .child(div().w_full().flex().flex_row().child(name))
                .child(detail_text(shown.size, ground)),
        )
        .on_click(cx.listener(move |view, event: &ClickEvent, _window, cx| {
            view.click_row(&path, event, cx);
        }));
    Some(kit.highlight(element, shown.picked).into_any_element())
}

fn kind_text(entry: &Entry) -> &'static str {
    kind_label(entry)
}

fn second_line(entry: &Entry, size: &str) -> String {
    let kind: Option<&str> = (!entry.opens_as_folder).then(|| kind_text(entry));
    kind.into_iter()
        .chain((!size.is_empty()).then_some(size))
        .collect::<Vec<_>>()
        .join(SECOND_LINE_JOIN)
}

fn cursor_wash(ground: Ground) -> Div {
    div()
        .absolute()
        .inset_0()
        .bg(rgba(translucent(ground.ink, Alpha::Tint)))
}

fn detail_text(text: String, ground: Ground) -> Div {
    div()
        .min_w_0()
        .text(TextStyle::Detail)
        .text_color(rgb(ground.soft))
        .child(text)
}

fn name_cell(
    name: &str,
    typed: Option<usize>,
    ground: Ground,
    style: TextStyle,
    centred: bool,
    window: &mut Window,
) -> Div {
    let cell = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_row()
        .when(centred, |cell| cell.justify_center());
    let Some(split) = typed else {
        return cell.child(name_text(name, style, ground.ink));
    };
    let (prefix, rest) = name.split_at(split);
    let rise = baseline(window, TextStyle::Title) - baseline(window, style);
    cell.h(px(line_height(style)))
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
        .child(name_text(rest, style, ground.ink))
}

fn name_text(name: &str, style: TextStyle, ink: u32) -> Div {
    div()
        .min_w_0()
        .text(style)
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
        .pl(px(SPACE_GUTTER + ICON + SPACE_CELL))
        .pr(px(SPACE_GUTTER))
        .text(TextStyle::Detail)
        .text_color(rgb(kit.grounds.pane.faint))
        .child(format!("Reading {name}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{kind_text, second_line};
    use crate::listing::{Entry, EntryKind, Size, SortKey};

    fn entry(name: &str, kind: EntryKind, opens_as_folder: bool) -> Entry {
        let os = OsString::from(name);
        Entry {
            display: name.to_string(),
            sort_key: SortKey::new(&os, opens_as_folder),
            name: os,
            path: PathBuf::from("/").join(name),
            id: None,
            kind,
            size: Size::Blank,
            hidden: false,
            opens_as_folder,
            executable: false,
        }
    }

    #[test]
    fn the_roomy_second_line_reads_kind_then_size_and_drops_what_is_missing() {
        let file = entry("notes.txt", EntryKind::File, false);
        let folder = entry("Music", EntryKind::Folder, true);
        let file_kind = kind_text(&file);
        let cases = [
            (&file, "4.2 KB", format!("{file_kind}  ·  4.2 KB")),
            (&file, "", file_kind.to_string()),
            (&folder, "3 items", "3 items".to_string()),
            (&folder, "", String::new()),
        ];
        for (entry, size, expected) in cases {
            assert_eq!(
                second_line(entry, size),
                expected,
                "{} {size}",
                entry.display
            );
        }
    }
}
