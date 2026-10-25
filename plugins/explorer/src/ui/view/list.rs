use std::ops::Range;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::{
    canvas, div, px, rgb, rgba, uniform_list, AnyElement, ClickEvent, Context, DispatchPhase, Div,
    ExternalPaths, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    ScrollWheelEvent, StatefulInteractiveElement, Styled, Window,
};
use qol_gpui::kit::{kit, Chip, Kit};
use qol_gpui::scrollbar::{ScrollGrip, ScrollSource};
use qol_gpui::text::{baseline, line_height, TextStyled};
use qol_gpui::theme::{
    translucent, Alpha, Ground, TextStyle, HEIGHT_INLINE, LINE, OPACITY_DISABLED, OPACITY_REST,
    SPACE_CELL, SPACE_GUTTER, SPACE_INSET, SPACE_STACK,
};

use super::drag;
use super::grid::{Layout, ACROSS};
use super::moves::progress_label;
use super::pictogram::entry_pictogram;
use super::press;
use super::previews;
use super::rename::row_name_room;
use super::targets::{self, Zone};
use super::{ExplorerView, HEAD, ICON};
use crate::listing::{Entry, ListingState};
use crate::ui::columns::{kind_label, left_to_right, size_label, typed_split};
use crate::ui::gesture::{HOLD, SLOP};

const KIND_WIDTH: f32 = 110.0;
const SIZE_WIDTH: f32 = 90.0;
pub(super) const NAME_MAX_WIDTH: f32 = 640.0;
pub(super) const COLUMNS_WIDTH: f32 = KIND_WIDTH + SIZE_WIDTH + 2.0 * SPACE_CELL;
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
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|view, event: &MouseDownEvent, _window, cx| view.context_down(event, cx)),
        )
        .on_drop(cx.listener(|view, paths: &ExternalPaths, _window, cx| {
            view.drop_here(paths.paths().to_vec(), cx);
        }))
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
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|view, event: &MouseDownEvent, _window, cx| view.press_down(event, cx)),
        )
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
        .children(view.marquee_box())
        .child(kit.scroll_cue(source.clone(), kit.grounds.pane))
        .child(ScrollGrip::new(
            "explorer-rows-grip",
            source,
            move |at, _window, cx| {
                let _ = seek.update(cx, |view, cx| view.seek_rows(at, cx));
            },
        ))
        .child(press::tracker(view.pressing.area(), cx))
        .into_any_element()
}

struct Shown<'a> {
    entry: &'a Entry,
    picked: bool,
    cursor: bool,
    ground: Ground,
    typed: Option<usize>,
    size: String,
    cut: bool,
    arriving: bool,
}

fn shown(view: &ExplorerView, row: usize, now: Instant) -> Option<Shown<'_>> {
    let browser = &view.browser;
    let entry = browser.entry(row)?;
    let picked = browser.is_picked(entry);
    let cursor = row == browser.cursor();
    let typed = cursor
        .then(|| typed_split(&entry.display, browser.jump_run(now)))
        .flatten();
    let arriving = browser.arriving_bytes(entry);
    let size = match arriving {
        Some((done, total)) => progress_label(done, total),
        None if view.hovered(Zone::Row(row)) => String::new(),
        None => size_label(
            entry,
            browser.listing().item_count(&entry.path),
            browser.shows_hidden(),
        ),
    };
    Some(Shown {
        entry,
        picked,
        cursor,
        ground: kit().highlight_ground(picked),
        typed,
        size,
        cut: browser.is_cut(entry),
        arriving: arriving.is_some(),
    })
}

fn fade(element: gpui::Stateful<Div>, cut: bool, arriving: bool) -> gpui::Stateful<Div> {
    if cut {
        element.opacity(OPACITY_DISABLED)
    } else if arriving {
        element.opacity(OPACITY_REST)
    } else {
        element
    }
}

fn folder_target(
    element: gpui::Stateful<Div>,
    entry: &Entry,
    row: usize,
    cx: &mut Context<ExplorerView>,
) -> gpui::Stateful<Div> {
    if !entry.opens_as_folder {
        return element;
    }
    targets::target(element, Zone::Row(row), entry.path.clone(), cx)
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
    let name = view
        .browser
        .renaming(entry)
        .then(|| {
            let room = row_name_room(view, stop.shows_columns());
            view.rename_label(room, stop.name_style(), ground, cx)
        })
        .flatten()
        .unwrap_or_else(|| name.into_any_element());
    let (cut, arriving) = (shown.cut, shown.arriving);
    let hovered = view.hovered(Zone::Row(row));
    let tag = view.drop_overlay(Zone::Row(row), ground);
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
        .when(shown.cursor || hovered, |element| {
            element.child(cursor_wash(ground))
        })
        .children(tag)
        .child(
            div().flex_none().w(px(ICON)).child(
                view.previews
                    .of(entry, ICON * window.scale_factor())
                    .map(|(shot, _)| previews::square(shot, ICON))
                    .unwrap_or_else(|| entry_pictogram(entry, stop.pictogram(), shown.picked)),
            ),
        )
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
                .child(if arriving {
                    div()
                        .flex_none()
                        .w(px(KIND_WIDTH))
                        .flex()
                        .child(kit.chip(Chip::Tag("arriving".into()), ground))
                } else {
                    detail_text(kind_text(entry).to_string(), ground)
                        .flex_none()
                        .w(px(KIND_WIDTH))
                })
                .child(
                    div()
                        .flex_none()
                        .w(px(SIZE_WIDTH))
                        .flex()
                        .justify_end()
                        .child(detail_text(shown.size, ground)),
                )
        })
        .drag_after_hold(HOLD, px(SLOP))
        .on_drag(path.clone(), {
            let view = cx.weak_entity();
            move |path, grab, window, cx| drag::start(&view, path, grab, window, cx)
        })
        .on_click(cx.listener(move |view, event: &ClickEvent, _window, cx| {
            view.click_row(&path, event, cx);
        }));
    let element = folder_target(fade(element, cut, arriving), entry, row, cx);
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
    let room = preview_room(width, height, stop.name_style());
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
    let name = view
        .browser
        .renaming(entry)
        .then(|| {
            let room = width - 2.0 * SPACE_INSET;
            view.rename_label(room, stop.name_style(), ground, cx)
        })
        .flatten()
        .unwrap_or_else(|| name.into_any_element());
    let (cut, arriving) = (shown.cut, shown.arriving);
    let hovered = view.hovered(Zone::Row(row));
    let tag = view.drop_overlay(Zone::Row(row), ground);
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
        .when(shown.cursor || hovered, |element| {
            element.child(cursor_wash(ground))
        })
        .children(tag)
        .child(
            view.previews
                .of(entry, room.0.max(room.1) * window.scale_factor())
                .map(|(shot, video)| previews::framed(shot, video, room))
                .unwrap_or_else(|| entry_pictogram(entry, stop.pictogram(), shown.picked)),
        )
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
        .drag_after_hold(HOLD, px(SLOP))
        .on_drag(path.clone(), {
            let view = cx.weak_entity();
            move |path, grab, window, cx| drag::start(&view, path, grab, window, cx)
        })
        .on_click(cx.listener(move |view, event: &ClickEvent, _window, cx| {
            view.click_row(&path, event, cx);
        }));
    let element = folder_target(fade(element, cut, arriving), entry, row, cx);
    Some(kit.highlight(element, shown.picked).into_any_element())
}

fn preview_room(width: f32, height: f32, name: TextStyle) -> (f32, f32) {
    let words = line_height(name) + SPACE_STACK + line_height(TextStyle::Detail);
    (
        width - 2.0 * SPACE_INSET,
        height - 3.0 * SPACE_INSET - words,
    )
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
            modified: None,
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
