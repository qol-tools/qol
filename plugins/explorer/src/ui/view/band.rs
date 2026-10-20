use std::path::PathBuf;

use gpui::prelude::FluentBuilder;
use gpui::{div, px, rgb, Context, IntoElement, ParentElement, Styled, Window};
use qol_gpui::kit::{kit, Kit};
use qol_gpui::settings_panel::components::settings_band_bar;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{TextStyle, SPACE_GUTTER, SPACE_SNUG};
use qol_gpui::CrumbTrail;

use super::{ExplorerView, BAND};
use crate::ui::browse::Note;
use crate::ui::trail;

const NOTE_WIDTH: f32 = 380.0;

pub(super) fn render(
    view: &ExplorerView,
    window: &Window,
    cx: &mut Context<ExplorerView>,
) -> impl IntoElement {
    let kit = kit();
    let browser = &view.browser;
    let note = browser.note();
    let editing = browser.path_edit();
    let shows_note = editing.is_none() || note.failure.is_some();
    let crumbs = trail::crumbs(
        browser.folder(),
        browser.home(),
        view.devices
            .iter()
            .map(|device| (device.label.as_str(), device.path.as_path())),
    );
    let paths: Vec<PathBuf> = crumbs.iter().map(|crumb| crumb.path.clone()).collect();
    let on_crumb = cx.entity().downgrade();
    let on_edit = on_crumb.clone();
    let on_suggestion = on_crumb.clone();
    let note_room = if shows_note {
        NOTE_WIDTH + SPACE_GUTTER
    } else {
        0.0
    };
    let width = f32::from(window.viewport_size().width) - 2.0 * SPACE_GUTTER - note_room;
    let mut crumb_trail = CrumbTrail::new(
        "explorer-trail",
        crumbs.into_iter().map(|crumb| crumb.label),
        kit,
    )
    .fit(width)
    .on_crumb(move |index, _window, cx| {
        let Some(folder) = paths.get(index).cloned() else {
            return;
        };
        let _ = on_crumb.update(cx, |view, cx| view.navigate(folder, cx));
    })
    .on_edit(move |_window, cx| {
        let _ = on_edit.update(cx, |view, cx| view.edit_path(cx));
    })
    .on_suggestion(move |index, _window, cx| {
        let _ = on_suggestion.update(cx, |view, cx| view.pick_suggestion(index, cx));
    });
    if let Some(edit) = editing {
        crumb_trail = crumb_trail.editing(edit, width);
    }
    settings_band_bar(BAND, kit)
        .child(div().flex_1().min_w_0().child(crumb_trail))
        .when(shows_note, |band| band.child(render_note(note, kit)))
}

fn render_note(note: Note, kit: Kit) -> impl IntoElement {
    let ground = kit.grounds.rail;
    div()
        .flex_none()
        .w(px(NOTE_WIDTH))
        .flex()
        .flex_row()
        .items_center()
        .justify_end()
        .gap(px(SPACE_SNUG))
        .when_some(note.failure, |row, failure| {
            row.child(
                div()
                    .min_w_0()
                    .text(TextStyle::Detail)
                    .text_color(rgb(kit.palette.danger))
                    .child(failure),
            )
        })
        .child(
            div()
                .flex_none()
                .text(TextStyle::Detail)
                .text_color(rgb(ground.faint))
                .child(note.count),
        )
}
