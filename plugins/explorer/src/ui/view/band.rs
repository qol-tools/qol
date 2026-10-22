use std::path::PathBuf;

use gpui::prelude::FluentBuilder;
use gpui::{div, px, rgb, AnyElement, Context, IntoElement, ParentElement, Styled, Window};
use qol_gpui::kit::{kit, Kit};
use qol_gpui::settings_panel::components::settings_band_bar;
use qol_gpui::surface::PanelDragArea;
use qol_gpui::text::{laid_out_width, TextStyled};
use qol_gpui::theme::{TextStyle, LINE, SPACE_GUTTER};
use qol_gpui::CrumbTrail;

use super::{slider, ExplorerView, BAND};
use crate::listing::FailureReason;
use crate::ui::browse::Note;
use crate::ui::columns::left_to_right;

const FAILURE_MAX_WIDTH: f32 = 380.0;
const TRAIL_FLOOR: f32 = 240.0;

pub(super) fn render(
    view: &ExplorerView,
    window: &mut Window,
    cx: &mut Context<ExplorerView>,
) -> impl IntoElement {
    let kit = kit();
    let browser = &view.browser;
    let note = browser.note();
    let editing = browser.path_edit();
    let room = f32::from(window.viewport_size().width)
        - 2.0 * (LINE + SPACE_GUTTER)
        - slider::WIDTH
        - SPACE_GUTTER;
    let limit = note_limit(room);
    let (note, note_width) = match editing {
        Some(_) => {
            let slot = reason_slot(window).min(limit);
            (render_reason(note.failure, slot, kit), slot)
        }
        None => {
            let widths = NoteWidths::measure(&note, window).fit(limit);
            (render_note(note, widths, kit), widths.total())
        }
    };
    let crumbs = view.crumbs();
    let paths: Vec<PathBuf> = crumbs.iter().map(|crumb| crumb.path.clone()).collect();
    let on_crumb = cx.entity().downgrade();
    let on_edit = on_crumb.clone();
    let on_suggestion = on_crumb.clone();
    let on_wheel = on_crumb.clone();
    let on_caret = on_crumb.clone();
    let on_folds = on_crumb.clone();
    let crumb_gesture = view.drag_gesture.clone();
    let edit_gesture = view.drag_gesture.clone();
    let folds_gesture = view.drag_gesture.clone();
    let note_room = if note_width > 0.0 {
        note_width + SPACE_GUTTER
    } else {
        0.0
    };
    let width = room - note_room;
    let mut crumb_trail = CrumbTrail::new(
        "explorer-trail",
        crumbs.iter().map(|crumb| left_to_right(&crumb.label)),
        kit,
    )
    .fit(width)
    .on_crumb(move |index, _window, cx| {
        if crumb_gesture.borrow().is_moving() {
            return;
        }
        let Some(folder) = paths.get(index).cloned() else {
            return;
        };
        let _ = on_crumb.update(cx, |view, cx| view.navigate(folder, cx));
    })
    .on_edit(move |_window, cx| {
        if edit_gesture.borrow().is_moving() {
            return;
        }
        let _ = on_edit.update(cx, |view, cx| view.edit_path(cx));
    })
    .on_suggestion(move |index, _window, cx| {
        let _ = on_suggestion.update(cx, |view, cx| view.pick_suggestion(index, cx));
    })
    .on_suggestion_wheel(move |rows, _window, cx| {
        let _ = on_wheel.update(cx, |view, cx| view.wheel_suggestions(rows, cx));
    })
    .on_caret(move |index, _window, cx| {
        let _ = on_caret.update(cx, |view, cx| view.place_caret(index, cx));
    })
    .folds(view.folds_open, move |open, _window, cx| {
        if open && folds_gesture.borrow().is_moving() {
            return;
        }
        let _ = on_folds.update(cx, |view, cx| view.show_folds(open, cx));
    });
    let band = settings_band_bar(BAND, kit);
    let band = match editing {
        Some(edit) => {
            crumb_trail = crumb_trail.editing(edit, width);
            band
        }
        None => band.panel_drag_after(&view.drag_gesture),
    };
    band.child(div().flex_1().min_w_0().child(crumb_trail))
        .when(note_width > 0.0, |band| band.child(note))
        .child(slider::render(view, cx))
}

#[derive(Clone, Copy)]
struct NoteWidths {
    failure: f32,
    count: f32,
}

impl NoteWidths {
    fn measure(note: &Note, window: &mut Window) -> Self {
        let failure = note.failure.as_deref().map_or(0.0, |failure| {
            laid_out_width(window, failure, TextStyle::Detail).min(FAILURE_MAX_WIDTH)
        });
        Self {
            failure,
            count: laid_out_width(window, &note.count, TextStyle::Detail),
        }
    }

    fn fit(self, limit: f32) -> Self {
        let count = self.count.min(limit);
        let failure_room = if count > 0.0 {
            limit - count - SPACE_GUTTER
        } else {
            limit
        };
        Self {
            failure: self.failure.min(failure_room.max(0.0)),
            count,
        }
    }

    fn total(self) -> f32 {
        let gap = if self.failure > 0.0 && self.count > 0.0 {
            SPACE_GUTTER
        } else {
            0.0
        };
        self.failure + gap + self.count
    }
}

fn note_limit(room: f32) -> f32 {
    (room - TRAIL_FLOOR - SPACE_GUTTER).max(0.0)
}

fn reason_slot(window: &mut Window) -> f32 {
    [
        FailureReason::Permission,
        FailureReason::Missing,
        FailureReason::NotAFolder,
        FailureReason::Other(String::new()),
    ]
    .iter()
    .map(|reason| laid_out_width(window, &reason.to_string(), TextStyle::Detail))
    .fold(0.0, f32::max)
}

fn render_reason(failure: Option<String>, slot: f32, kit: Kit) -> AnyElement {
    div()
        .flex_none()
        .w(px(slot))
        .flex()
        .flex_row()
        .justify_end()
        .when_some(failure, |row, failure| {
            row.child(failure_text(failure, kit))
        })
        .into_any_element()
}

fn render_note(note: Note, widths: NoteWidths, kit: Kit) -> AnyElement {
    div()
        .flex_none()
        .w(px(widths.total()))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(SPACE_GUTTER))
        .when_some(
            note.failure.filter(|_| widths.failure > 0.0),
            |row, failure| row.child(failure_text(failure, kit).w(px(widths.failure))),
        )
        .when(widths.count > 0.0, |row| {
            row.child(
                div()
                    .flex_none()
                    .w(px(widths.count))
                    .text(TextStyle::Detail)
                    .text_color(rgb(kit.grounds.rail.faint))
                    .child(note.count),
            )
        })
        .into_any_element()
}

fn failure_text(failure: String, kit: Kit) -> gpui::Div {
    div()
        .min_w_0()
        .text(TextStyle::Detail)
        .text_color(rgb(kit.palette.danger_ink))
        .child(failure)
}

#[cfg(test)]
mod tests {
    use qol_gpui::theme::SPACE_GUTTER;

    use super::{note_limit, NoteWidths, TRAIL_FLOOR};

    #[test]
    fn the_note_gives_way_so_the_trail_keeps_its_floor() {
        assert_eq!(note_limit(TRAIL_FLOOR), 0.0);
        assert_eq!(note_limit(TRAIL_FLOOR + SPACE_GUTTER + 150.0), 150.0);
        let cases = [
            ((380.0, 80.0), 1000.0, (380.0, 80.0)),
            ((380.0, 80.0), 300.0, (300.0 - 80.0 - SPACE_GUTTER, 80.0)),
            ((380.0, 80.0), 90.0, (0.0, 80.0)),
            ((380.0, 80.0), 50.0, (0.0, 50.0)),
            ((380.0, 0.0), 120.0, (120.0, 0.0)),
            ((0.0, 80.0), 0.0, (0.0, 0.0)),
        ];
        for ((failure, count), limit, (fit_failure, fit_count)) in cases {
            let fitted = NoteWidths { failure, count }.fit(limit);
            assert_eq!(
                (fitted.failure, fitted.count),
                (fit_failure, fit_count),
                "{failure} {count} {limit}"
            );
            assert!(fitted.total() <= limit, "{limit}");
        }
    }
}
