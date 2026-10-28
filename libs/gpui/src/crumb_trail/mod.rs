use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    anchored, canvas, deferred, div, font, px, relative, rgb, rgba, AnyElement, App, Bounds, Div,
    ElementId, Entity, Font, MouseButton, Pixels, Rgba, SharedString, Window,
};
use qol_theme::{translucent, Alpha, TextStyle};

use crate::kit::Kit;
use crate::scroll_list::wheel_rows;
use crate::scrollbar::ScrollSource;
use crate::text::{laid_out_width, shaped_width, TextStyled};
use crate::text_edit::{visible_char_count, CaretStyle, TextFieldElement};

mod path_edit;

pub use path_edit::{PathEdit, PathKey};

pub const CRUMB_MAX_WIDTH: f32 = 200.0;
pub const SUGGESTION_ROWS: usize = 8;
const CARET_WIDTH: f32 = 2.0;

pub type CrumbClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub type EditRequest = Rc<dyn Fn(&mut Window, &mut App)>;
pub type SuggestionWheel = Rc<dyn Fn(isize, &mut Window, &mut App)>;
pub type FoldToggle = Rc<dyn Fn(bool, &mut Window, &mut App)>;

const FOLD_MENU_WIDTH: f32 =
    CRUMB_MAX_WIDTH + 2.0 * (qol_theme::SPACE_INSET + qol_theme::SPACE_TIGHT);

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
    on_suggestion_wheel: Option<SuggestionWheel>,
    on_caret: Option<CrumbClick>,
    folds: Option<Folds>,
    kit: Kit,
}

struct Folds {
    open: bool,
    on_toggle: FoldToggle,
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
            on_suggestion_wheel: None,
            on_caret: None,
            folds: None,
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

    pub fn on_suggestion_wheel(
        mut self,
        on_wheel: impl Fn(isize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_suggestion_wheel = Some(Rc::new(on_wheel));
        self
    }

    pub fn on_caret(mut self, on_caret: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_caret = Some(Rc::new(on_caret));
        self
    }

    pub fn folds(
        mut self,
        open: bool,
        on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.folds = Some(Folds {
            open,
            on_toggle: Rc::new(on_toggle),
        });
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let CrumbTrail {
            id,
            crumbs,
            editing,
            fit,
            on_crumb,
            on_edit,
            on_suggestion,
            on_suggestion_wheel,
            on_caret,
            folds,
            kit,
        } = self;
        match editing {
            Some(Editing { edit, width }) => path_field(
                &edit,
                width,
                Handlers {
                    on_suggestion,
                    on_wheel: on_suggestion_wheel,
                    on_caret,
                },
                kit,
                window,
            ),
            None => {
                let folded = fit.map_or(0..0, |width| folded_run(&crumbs, width, window));
                let hidden: Vec<(usize, SharedString)> = folded
                    .clone()
                    .filter_map(|index| Some((index, crumbs.get(index)?.clone())))
                    .collect();
                let mark = fold_mark(&id, hidden, on_crumb.clone(), folds, kit, window, cx);
                read_only(id, crumbs, folded, mark, on_crumb, on_edit, kit)
            }
        }
    }
}

fn folded_run(crumbs: &[SharedString], width: f32, window: &mut Window) -> Range<usize> {
    let mut measure = |text: &str| laid_out_width(window, text, TextStyle::Heading);
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
    mark: AnyElement,
    on_crumb: Option<CrumbClick>,
    on_edit: Option<EditRequest>,
    kit: Kit,
) -> AnyElement {
    let last = crumbs.len().saturating_sub(1);
    let divider = rgba(translucent(kit.grounds.pane.faint, Alpha::Veil));
    let mut children = Vec::with_capacity(crumbs.len() * 2);
    let faint = rgb(kit.grounds.pane.faint);
    let mut mark = Some(mark);
    for (index, label) in crumbs.into_iter().enumerate() {
        if folded.contains(&index) {
            if let Some(mark) = mark.take() {
                children.push(separator(divider));
                children.push(mark);
            }
            continue;
        }
        if index > 0 {
            children.push(separator(divider));
        }
        let crumb = if index == last {
            div().min_w_0().text_color(rgb(kit.grounds.pane.ink))
        } else {
            div()
                .flex_none()
                .max_w(px(CRUMB_MAX_WIDTH))
                .text_color(faint)
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

fn fold_mark(
    id: &ElementId,
    hidden: Vec<(usize, SharedString)>,
    on_crumb: Option<CrumbClick>,
    folds: Option<Folds>,
    kit: Kit,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let mark = div()
        .relative()
        .flex_none()
        .text_color(rgb(kit.grounds.pane.faint))
        .child("…");
    let (Some(Folds { open, on_toggle }), Some(on_crumb)) = (folds, on_crumb) else {
        return mark.into_any_element();
    };
    if hidden.is_empty() {
        if open {
            window.defer(cx, move |window, cx| on_toggle(false, window, cx));
        }
        return mark.into_any_element();
    }
    let first = window.use_keyed_state(
        ElementId::NamedChild(Box::new(id.clone()), "folds".into()),
        cx,
        |_, _| 0usize,
    );
    let mark_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let measured = mark_bounds.clone();
    let toggle = on_toggle.clone();
    let reset = first.clone();
    let menu = open.then(|| {
        let pick: CrumbClick = Rc::new(move |index, window, cx| {
            on_toggle(false, window, cx);
            on_crumb(index, window, cx);
        });
        fold_menu(hidden, first, mark_bounds, toggle.clone(), pick, kit, cx)
    });
    mark.id("folds")
        .cursor_pointer()
        .child(
            canvas(
                move |bounds, _, _| measured.set(Some(bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            if !open {
                reset.update(cx, |first, _| *first = 0);
            }
            toggle(!open, window, cx);
        })
        .when_some(menu, |mark, menu| {
            mark.child(
                div()
                    .absolute()
                    .left_0()
                    .top(relative(1.0))
                    .pt(px(qol_theme::SPACE_TIGHT))
                    .child(deferred(anchored().child(menu))),
            )
        })
        .into_any_element()
}

fn fold_menu(
    hidden: Vec<(usize, SharedString)>,
    first: Entity<usize>,
    mark_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    on_toggle: FoldToggle,
    pick: CrumbClick,
    kit: Kit,
    cx: &mut App,
) -> Div {
    let total = hidden.len();
    let top = (*first.read(cx)).min(total.saturating_sub(SUGGESTION_ROWS));
    let rows: Vec<AnyElement> = hidden
        .iter()
        .skip(top)
        .take(SUGGESTION_ROWS)
        .map(|(index, label)| {
            menu_row(
                ("fold", *index).into(),
                label,
                *index,
                false,
                Some(pick.clone()),
                kit,
            )
        })
        .collect();
    let shown = rows.len();
    kit.menu(FOLD_MENU_WIDTH)
        .children(rows)
        .child(kit.scroll_cue(
            ScrollSource::Window {
                first: top,
                shown,
                total,
            },
            kit.grounds.menu,
        ))
        .on_scroll_wheel(move |event, _, cx| {
            cx.stop_propagation();
            let rows = wheel_rows(&event.delta, qol_theme::HEIGHT_INLINE);
            let moved = top
                .saturating_add_signed(rows)
                .min(total.saturating_sub(SUGGESTION_ROWS));
            if moved != top {
                first.update(cx, |first, cx| {
                    *first = moved;
                    cx.notify();
                });
            }
        })
        .on_mouse_down_out(move |event, window, cx| {
            if mark_bounds
                .get()
                .is_some_and(|bounds| bounds.contains(&event.position))
            {
                return;
            }
            on_toggle(false, window, cx);
        })
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

struct Handlers {
    on_suggestion: Option<CrumbClick>,
    on_wheel: Option<SuggestionWheel>,
    on_caret: Option<CrumbClick>,
}

fn path_field(
    edit: &PathEdit,
    width: f32,
    handlers: Handlers,
    kit: Kit,
    window: &mut Window,
) -> AnyElement {
    let pane = kit.grounds.pane;
    let size = TextStyle::Code.spec().size;
    let mono = font(qol_theme::font_mono());
    let advance = shaped_width(window, "0", mono.clone(), size);
    let visible = visible_char_count(width - 2.0 * qol_theme::SPACE_CELL - CARET_WIDTH, advance);
    let mut text = TextFieldElement::new(edit.field(), visible, advance)
        .with_marks()
        .measure(|shown| shaped_width(window, shown, font(qol_theme::font_mono()), size))
        .selection(rgb(kit.grounds.band.bg).into(), Some(rgb(pane.ink).into()))
        .caret(CaretStyle {
            color: rgb(pane.ink).into(),
            width: CARET_WIDTH,
            height: 16.0,
            top: 1.0,
            radius: 1.0,
        });
    let (start, shown) = text.shown_window();
    let text = text.render();
    let text_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let measured = text_bounds.clone();
    let text = div()
        .relative()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .child(text)
        .child(
            canvas(
                move |bounds, _, _| measured.set(Some(bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
    let field = div()
        .flex()
        .items_center()
        .w(px(width))
        .h(px(qol_theme::HEIGHT_CONTROL))
        .px(px(qol_theme::SPACE_CELL))
        .rounded(px(qol_theme::RADIUS_CONTROL))
        .bg(rgba(pane.well.packed()))
        .map(|field| kit.ringed(field, pane, qol_theme::RADIUS_CONTROL))
        .text(TextStyle::Code)
        .text_color(rgb(pane.ink))
        .child(text)
        .when_some(handlers.on_caret, |field, on_caret| {
            field
                .cursor_text()
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    let Some(bounds) = text_bounds.get() else {
                        return;
                    };
                    cx.stop_propagation();
                    let x = f32::from(event.position.x - bounds.origin.x);
                    let offset = char_at(window, &shown, x, mono.clone(), size);
                    on_caret(start + offset, window, cx);
                })
        });
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .child(field)
        .when_some(
            suggestion_menu(edit, width, handlers.on_suggestion, handlers.on_wheel, kit),
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

fn char_at(window: &mut Window, shown: &str, x: f32, mono: Font, size: f32) -> usize {
    let mut before = 0.0;
    for (offset, (index, letter)) in shown.char_indices().enumerate() {
        let after = shaped_width(
            window,
            &shown[..index + letter.len_utf8()],
            mono.clone(),
            size,
        );
        if x < (before + after) / 2.0 {
            return offset;
        }
        before = after;
    }
    shown.chars().count()
}

fn suggestion_menu(
    edit: &PathEdit,
    width: f32,
    on_suggestion: Option<CrumbClick>,
    on_wheel: Option<SuggestionWheel>,
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
            menu_row(
                ("suggestion", index).into(),
                name,
                index,
                selected == Some(index),
                on_suggestion.clone(),
                kit,
            )
        });
    Some(
        kit.menu(width)
            .children(rows)
            .child(kit.scroll_cue(
                ScrollSource::Window {
                    first,
                    shown: count,
                    total: edit.suggestions().len(),
                },
                kit.grounds.menu,
            ))
            .when_some(on_wheel, |menu, on_wheel| {
                menu.on_scroll_wheel(move |event, window, cx| {
                    cx.stop_propagation();
                    let rows = wheel_rows(&event.delta, qol_theme::HEIGHT_INLINE);
                    if rows != 0 {
                        on_wheel(rows, window, cx);
                    }
                })
            }),
    )
}

fn menu_row(
    id: ElementId,
    name: &str,
    index: usize,
    selected: bool,
    on_click: Option<CrumbClick>,
    kit: Kit,
) -> AnyElement {
    let row = kit.menu_row(id, name.to_owned(), None, selected);
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
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::prelude::*;
    use gpui::{div, Context, TestAppContext, Window};
    use qol_theme::{ThemeMode, LIGHT_SYSTEM};

    use super::{fold, CrumbTrail};
    use crate::kit::Kit;

    struct OpenFolds {
        fit: f32,
        closed: Rc<Cell<bool>>,
    }

    impl Render for OpenFolds {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let closed = self.closed.clone();
            div().child(
                CrumbTrail::new(
                    "trail",
                    ["home", "qol", "deep", "leaf"],
                    Kit::new(ThemeMode::Light, LIGHT_SYSTEM),
                )
                .fit(self.fit)
                .on_crumb(|_, _, _| {})
                .folds(true, move |open, _, _| closed.set(!open)),
            )
        }
    }

    #[gpui::test]
    fn an_open_fold_menu_closes_once_nothing_is_folded(cx: &mut TestAppContext) {
        for (fit, closes) in [(0.0, false), (f32::MAX, true)] {
            let closed = Rc::new(Cell::new(false));
            let seen = closed.clone();
            cx.add_window_view(|_, _| OpenFolds { fit, closed: seen });
            assert_eq!(closed.get(), closes, "fit {fit}");
        }
    }

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
