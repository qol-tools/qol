use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    anchored, deferred, div, font, px, rgb, rgba, AnyElement, App, Div, ElementId, FontWeight,
    Rgba, SharedString, Window,
};
use qol_theme::{translucent, Alpha, TextStyle};

use crate::kit::{float_shadow, Kit};
use crate::scrollbar::ScrollSource;
use crate::text::{shaped_width, TextStyled};
use crate::text_edit::{visible_char_count, CaretStyle, TextFieldElement};

mod path_edit;

pub use path_edit::{PathEdit, PathKey};

pub const CRUMB_MAX_WIDTH: f32 = 200.0;
pub const SUGGESTION_ROWS: usize = 8;
const CARET_WIDTH: f32 = 2.0;

pub type CrumbClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub type EditRequest = Rc<dyn Fn(&mut Window, &mut App)>;

struct Editing {
    edit: PathEdit,
    width: f32,
}

#[derive(IntoElement)]
pub struct CrumbTrail {
    id: ElementId,
    crumbs: Vec<SharedString>,
    editing: Option<Editing>,
    fit: Option<f32>,
    on_crumb: Option<CrumbClick>,
    on_edit: Option<EditRequest>,
    on_suggestion: Option<CrumbClick>,
    kit: Kit,
}

impl CrumbTrail {
    pub fn new(
        id: impl Into<ElementId>,
        crumbs: impl IntoIterator<Item = impl Into<SharedString>>,
        kit: Kit,
    ) -> Self {
        Self {
            id: id.into(),
            crumbs: crumbs.into_iter().map(Into::into).collect(),
            editing: None,
            fit: None,
            on_crumb: None,
            on_edit: None,
            on_suggestion: None,
            kit,
        }
    }

    pub fn on_crumb(mut self, on_crumb: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_crumb = Some(Rc::new(on_crumb));
        self
    }

    pub fn on_edit(mut self, on_edit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_edit = Some(Rc::new(on_edit));
        self
    }

    pub fn on_suggestion(
        mut self,
        on_suggestion: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_suggestion = Some(Rc::new(on_suggestion));
        self
    }

    pub fn fit(mut self, width: f32) -> Self {
        self.fit = Some(width);
        self
    }

    pub fn editing(mut self, edit: &PathEdit, width: f32) -> Self {
        self.editing = Some(Editing {
            edit: edit.clone(),
            width,
        });
        self
    }
}

impl RenderOnce for CrumbTrail {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let CrumbTrail {
            id,
            crumbs,
            editing,
            fit,
            on_crumb,
            on_edit,
            on_suggestion,
            kit,
        } = self;
        match editing {
            Some(Editing { edit, width }) => path_field(&edit, width, on_suggestion, kit, window),
            None => {
                let folded = fit.map_or(0..0, |width| folded_run(&crumbs, width, window));
                read_only(id, crumbs, folded, on_crumb, on_edit, kit)
            }
        }
    }
}

fn folded_run(crumbs: &[SharedString], width: f32, window: &mut Window) -> Range<usize> {
    let spec = TextStyle::Heading.spec();
    let mut face = font(spec.face.family());
    face.weight = FontWeight(f32::from(spec.weight));
    let mut measure = |text: &str| shaped_width(window, text, face.clone(), spec.size);
    let separator = measure("/") + 2.0 * qol_theme::SPACE_TIGHT;
    let last = crumbs.len().saturating_sub(1);
    let widths: Vec<f32> = crumbs
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let label = measure(label);
            if index == last {
                label
            } else {
                label.min(CRUMB_MAX_WIDTH)
            }
        })
        .collect();
    let ellipsis = measure("…");
    fold(&widths, separator, ellipsis, width)
}

fn fold(widths: &[f32], separator: f32, ellipsis: f32, width: f32) -> Range<usize> {
    let last = widths.len().saturating_sub(1);
    let mut shown = widths.iter().sum::<f32>() + separator * last as f32;
    if shown <= width || widths.len() < 3 {
        return 0..0;
    }
    shown += ellipsis + separator;
    let mut end = 1;
    while end < last && shown > width {
        shown -= widths[end] + separator;
        end += 1;
    }
    1..end
}

fn separator(color: Rgba) -> AnyElement {
    div()
        .flex_none()
        .px(px(qol_theme::SPACE_TIGHT))
        .text_color(color)
        .child("/")
        .into_any_element()
}

fn read_only(
    id: ElementId,
    crumbs: Vec<SharedString>,
    folded: Range<usize>,
    on_crumb: Option<CrumbClick>,
    on_edit: Option<EditRequest>,
    kit: Kit,
) -> AnyElement {
    let last = crumbs.len().saturating_sub(1);
    let divider = rgba(translucent(kit.grounds.pane.faint, Alpha::Veil));
    let mut children = Vec::with_capacity(crumbs.len() * 2);
    let faint = rgb(kit.grounds.pane.faint);
    for (index, label) in crumbs.into_iter().enumerate() {
        if folded.contains(&index) {
            if index == folded.start {
                children.push(separator(divider));
                children.push(
                    div()
                        .flex_none()
                        .text_color(faint)
                        .child("…")
                        .into_any_element(),
                );
            }
            continue;
        }
        if index > 0 {
            children.push(separator(divider));
        }
        let crumb = if index == last {
            div().min_w_0().text_color(rgb(kit.grounds.pane.ink))
        } else {
            div().max_w(px(CRUMB_MAX_WIDTH)).text_color(faint)
        };
        let crumb = crumb.child(label);
        children.push(match &on_crumb {
            Some(on_crumb) => clickable_crumb(crumb, index, on_crumb.clone()),
            None => crumb.into_any_element(),
        });
    }
    let trail = kit.crumb_row().children(children);
    if on_crumb.is_none() && on_edit.is_none() {
        return trail.into_any_element();
    }
    let trail = trail.id(id);
    match on_edit {
        Some(on_edit) => trail
            .flex_1()
            .cursor_text()
            .on_click(move |_, window, cx| on_edit(window, cx))
            .into_any_element(),
        None => trail.into_any_element(),
    }
}

fn clickable_crumb(crumb: Div, index: usize, on_crumb: CrumbClick) -> AnyElement {
    crumb
        .id(index)
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_crumb(index, window, cx);
        })
        .into_any_element()
}

fn path_field(
    edit: &PathEdit,
    width: f32,
    on_suggestion: Option<CrumbClick>,
    kit: Kit,
    window: &mut Window,
) -> AnyElement {
    let pane = kit.grounds.pane;
    let size = TextStyle::Code.spec().size;
    let advance = shaped_width(window, "0", font(qol_theme::font_mono()), size);
    let visible = visible_char_count(width - 2.0 * qol_theme::SPACE_CELL - CARET_WIDTH, advance);
    let text = TextFieldElement::new(edit.field(), visible, advance)
        .with_marks()
        .measure_caret(|before| shaped_width(window, before, font(qol_theme::font_mono()), size))
        .selection(rgb(kit.grounds.band.bg).into(), Some(rgb(pane.ink).into()))
        .caret(CaretStyle {
            color: rgb(pane.ink).into(),
            width: CARET_WIDTH,
            height: 16.0,
            top: 1.0,
            radius: 1.0,
        })
        .render();
    let field = div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(qol_theme::HEIGHT_CONTROL))
        .px(px(qol_theme::SPACE_CELL))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .bg(rgba(pane.well.packed()))
        .shadow(kit.focus_ring(pane))
        .text(TextStyle::Code)
        .text_color(rgb(pane.ink))
        .overflow_hidden()
        .child(div().flex_1().min_w_0().child(text));
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .child(field)
        .when_some(
            suggestion_menu(edit, width, on_suggestion, kit),
            |slot, menu| {
                slot.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(qol_theme::HEIGHT_CONTROL + qol_theme::SPACE_TIGHT))
                        .child(deferred(anchored().child(menu))),
                )
            },
        )
        .into_any_element()
}

fn suggestion_menu(
    edit: &PathEdit,
    width: f32,
    on_suggestion: Option<CrumbClick>,
    kit: Kit,
) -> Option<Div> {
    if edit.suggestions().is_empty() {
        return None;
    }
    let selected = edit.selected();
    let shown = edit.shown(SUGGESTION_ROWS);
    let first = shown.start;
    let count = shown.len();
    let rows = edit.suggestions()[shown]
        .iter()
        .enumerate()
        .map(|(offset, name)| {
            let index = first + offset;
            suggestion_row(
                name,
                index,
                selected == Some(index),
                on_suggestion.clone(),
                kit,
            )
        });
    Some(
        div()
            .occlude()
            .relative()
            .w(px(width))
            .p(px(qol_theme::SPACE_TIGHT))
            .border(px(qol_theme::LINE))
            .border_color(rgba(kit.washes.hairline_strong.packed()))
            .bg(rgb(kit.grounds.menu.bg))
            .shadow(float_shadow(kit.washes.cast.rgb))
            .flex()
            .flex_col()
            .children(rows)
            .child(kit.scroll_cue(
                ScrollSource::Window {
                    first,
                    shown: count,
                    total: edit.suggestions().len(),
                },
                kit.grounds.menu,
            )),
    )
}

fn suggestion_row(
    name: &str,
    index: usize,
    selected: bool,
    on_click: Option<CrumbClick>,
    kit: Kit,
) -> AnyElement {
    let row = div()
        .id(("suggestion", index))
        .h(px(qol_theme::HEIGHT_INLINE))
        .px(px(qol_theme::SPACE_INSET))
        .flex()
        .items_center()
        .text(TextStyle::ListName)
        .text_color(rgb(kit.highlight_ground(selected).ink))
        .child(div().flex_1().min_w_0().child(name.to_owned()));
    let row = kit.highlight(row, selected);
    match on_click {
        Some(on_click) => row
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_click(index, window, cx);
            })
            .into_any_element(),
        None => row.into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn a_trail_that_fits_folds_nothing() {
        assert_eq!(fold(&[50.0, 50.0, 50.0], 10.0, 10.0, 170.0), 0..0);
        assert_eq!(fold(&[500.0, 500.0], 10.0, 10.0, 100.0), 0..0);
    }

    #[test]
    fn an_overflowing_trail_folds_the_crumbs_after_the_first_until_the_rest_fit() {
        let widths = [40.0; 10];
        assert_eq!(fold(&widths, 10.0, 10.0, 200.0), 1..8);
        assert_eq!(fold(&widths, 10.0, 10.0, 0.0), 1..9);
    }
}
