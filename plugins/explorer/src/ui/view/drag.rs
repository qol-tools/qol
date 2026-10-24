use std::path::{Path, PathBuf};

use gpui::prelude::FluentBuilder;
use gpui::{
    div, point, px, rgb, AnimationExt as _, AnyElement, App, AppContext, Context, Div, DragImage,
    Entity, IntoElement, ParentElement, Pixels, Point, Render, Styled, WeakEntity, Window,
};
use qol_gpui::kit::{kit, CHIP_COUNT_HEIGHT};
use qol_gpui::motion;
use qol_gpui::text::TextStyled;
use qol_gpui::theme::{count_disc, font_ui, Motion, TextStyle, LINE, SPACE_TIGHT};

use super::pictogram::{lifted_pictogram, lifted_source, Lifted};
use super::ExplorerView;
use crate::listing::Entry;
use crate::ui::browse::Browser;
use crate::ui::gesture::{charge_look, diameter, label_opacity};

const BESIDE_POINTER: f32 = 14.0;
const ICON: f32 = 40.0;
const STEP: f32 = 6.0;
const STACKED: usize = 3;
pub(super) const COUNT_FROM: usize = 3;
const COUNT_LEFT: f32 = 29.0;
const COUNT_RISE: f32 = 4.0;
const DIGIT_WIDTH: f32 = 7.0;
const SETTLE_FROM: f32 = 1.4;
const CHARGE_RING: f32 = 2.0 * LINE;

pub(super) struct Carry {
    pub(super) count: usize,
    pub(super) stack: Vec<Entry>,
}

pub(super) struct DraggedPaths {
    count: usize,
    stack: Vec<Entry>,
    grab: Point<Pixels>,
}

impl Render for DraggedPaths {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .pl(self.grab.x + px(BESIDE_POINTER))
            .pt(self.grab.y + px(BESIDE_POINTER))
            .child(stack_view(&self.stack, self.count, 1.0))
    }
}

fn stack_view(stack: &[Entry], count: usize, disc_scale: f32) -> Div {
    let kit = kit();
    let behind = behind(stack.len());
    let icons = stack.iter().enumerate().rev().map(|(depth, entry)| {
        let offset = depth as f32 * STEP;
        div()
            .absolute()
            .left(px(offset))
            .top(px(COUNT_RISE + behind - offset))
            .child(lifted_pictogram(entry, ICON))
    });
    let grow = disc_scale - 1.0;
    let width = disc_width(count);
    let disc = (count >= COUNT_FROM).then(|| {
        div()
            .absolute()
            .left(px(behind + COUNT_LEFT - grow * width / 2.0))
            .top(px(-grow * CHIP_COUNT_HEIGHT / 2.0))
            .child(
                kit.count_disc(count, kit.grounds.pane)
                    .when(grow > 0.0, |disc| {
                        disc.min_w(px(width * disc_scale))
                            .h(px(CHIP_COUNT_HEIGHT * disc_scale))
                    }),
            )
    });
    div()
        .relative()
        .w(px(ICON + behind + COUNT_LEFT))
        .h(px(ICON + behind + COUNT_RISE))
        .children(icons)
        .children(disc)
}

pub(super) fn carry(browser: &Browser, path: &Path, additive: bool) -> Carry {
    let paths = browser.carried(path, additive);
    Carry {
        count: paths.len(),
        stack: stack_of(browser.listing().entries(), path, &paths),
    }
}

fn stack_of(entries: &[Entry], path: &Path, paths: &[PathBuf]) -> Vec<Entry> {
    std::iter::once(path)
        .chain(
            paths
                .iter()
                .map(|path| path.as_path())
                .filter(|&other| other != path),
        )
        .filter_map(|path| entries.iter().find(|entry| entry.path == path).cloned())
        .take(STACKED)
        .collect()
}

pub(super) fn lifted(carry: &Carry, at: Point<Pixels>, serial: u64) -> AnyElement {
    let stack = carry.stack.clone();
    let count = carry.count;
    let placed = move |disc_scale: f32| {
        div()
            .absolute()
            .left(at.x + px(BESIDE_POINTER))
            .top(at.y + px(BESIDE_POINTER))
            .child(stack_view(&stack, count, disc_scale))
    };
    if count < COUNT_FROM {
        return placed(1.0).into_any_element();
    }
    div()
        .with_animation(
            ("explorer-lift", serial as usize),
            motion::animation(Motion::QUICK),
            move |_, delta| placed(SETTLE_FROM - (SETTLE_FROM - 1.0) * delta),
        )
        .into_any_element()
}

pub(super) fn charge(carry: &Carry, at: Point<Pixels>, progress: f32) -> AnyElement {
    let pane = kit().grounds.pane;
    let (across, down) = disc_centre(carry.count, carry.stack.len());
    let (scale, opacity) = charge_look(motion::ease(Motion::TRAVEL, progress));
    let size = diameter(carry.count) * scale;
    let label = if carry.count >= COUNT_FROM {
        label_opacity(progress)
    } else {
        0.0
    };
    div()
        .absolute()
        .left(at.x + px(across - size / 2.0))
        .top(at.y + px(down - size / 2.0))
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border(px(CHARGE_RING))
        .border_color(rgb(pane.bg))
        .bg(rgb(count_disc(pane)))
        .opacity(opacity)
        .when(label > 0.0, |circle| {
            circle.child(
                div()
                    .opacity(label)
                    .text(TextStyle::Label)
                    .text_color(rgb(pane.on_mark))
                    .child(carry.count.to_string()),
            )
        })
        .into_any_element()
}

pub(super) fn disc_width(count: usize) -> f32 {
    let digits = count.to_string().len();
    (digits as f32 * DIGIT_WIDTH + SPACE_TIGHT * 2.0).max(CHIP_COUNT_HEIGHT)
}

pub(super) fn disc_centre(count: usize, depth: usize) -> (f32, f32) {
    (
        BESIDE_POINTER + behind(depth) + COUNT_LEFT + disc_width(count) / 2.0,
        BESIDE_POINTER + CHIP_COUNT_HEIGHT / 2.0,
    )
}

pub(super) fn start(
    view: &WeakEntity<ExplorerView>,
    path: &Path,
    grab: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DraggedPaths> {
    let (paths, stack) = view
        .upgrade()
        .map(|view| {
            let handed = view.update(cx, |view, cx| view.hand_over(cx));
            let browser = &view.read(cx).browser;
            let paths = browser.drag_paths(path);
            let stack =
                handed.unwrap_or_else(|| stack_of(browser.listing().entries(), path, &paths));
            (paths, stack)
        })
        .unwrap_or_else(|| (vec![path.to_path_buf()], Vec::new()));
    let count = paths.len();
    let cancelled = view
        .upgrade()
        .is_some_and(|view| view.read(cx).press_was_cancelled());
    if cancelled {
        window.defer(cx, |window, cx| {
            cx.stop_active_drag(window);
        });
    } else {
        let image = drag_image(&stack, count, window.scale_factor());
        window.offer_dragged_paths(paths, image);
    }
    cx.new(|_| DraggedPaths { count, stack, grab })
}

fn behind(stacked: usize) -> f32 {
    stacked.saturating_sub(1) as f32 * STEP
}

fn drag_image(stack: &[Entry], count: usize, scale: f32) -> Option<DragImage> {
    let sources = stack.iter().map(lifted_source).collect::<Vec<_>>();
    let (name, markup) = stack_markup(&sources, count);
    let side = ICON + behind(sources.len()) + COUNT_LEFT + CHIP_COUNT_HEIGHT * 2.0;
    let image = qol_gpui::pictures::icon(
        &name,
        &markup.replace("{side}", &side.to_string()),
        0,
        (side * scale).round() as u32,
    )?;
    Some(DragImage {
        image,
        offset: point(px(BESIDE_POINTER), px(BESIDE_POINTER)),
    })
}

fn stack_markup(sources: &[Lifted], count: usize) -> (String, String) {
    let behind = behind(sources.len());
    let layers = sources
        .iter()
        .enumerate()
        .rev()
        .map(|(depth, source)| {
            let offset = depth as f32 * STEP;
            let placed = format!(
                r#"<svg x="{offset}" y="{}" width="{ICON}" height="{ICON}" "#,
                COUNT_RISE + behind - offset
            );
            source
                .markup
                .replace("currentColor", &format!("#{:06x}", source.ink))
                .replacen("<svg ", &placed, 1)
        })
        .collect::<String>();
    let disc = if count >= COUNT_FROM {
        count_markup(count, behind + COUNT_LEFT)
    } else {
        String::new()
    };
    let names = sources
        .iter()
        .map(|source| source.name.as_str())
        .collect::<Vec<_>>()
        .join("+");
    (
        format!("explorer-drag-{names}-{count}"),
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {{side}} {{side}}">{layers}{disc}</svg>"#
        ),
    )
}

fn count_markup(count: usize, left: f32) -> String {
    let pane = kit().grounds.pane;
    let label = TextStyle::Label.spec();
    let text = count.to_string();
    let width = disc_width(count);
    let radius = CHIP_COUNT_HEIGHT / 2.0;
    format!(
        r##"<rect x="{left}" y="0" width="{width}" height="{CHIP_COUNT_HEIGHT}" rx="{radius}" fill="#{:06x}"/><text x="{}" y="{}" fill="#{:06x}" font-family="{}" font-weight="{}" font-size="{}" text-anchor="middle">{text}</text>"##,
        count_disc(pane),
        left + width / 2.0,
        radius + label.size * 0.35,
        pane.on_mark,
        font_ui(),
        label.weight,
        label.size,
    )
}

#[cfg(test)]
mod tests {
    use super::{disc_centre, disc_width, BESIDE_POINTER, CHIP_COUNT_HEIGHT, COUNT_LEFT, STEP};

    #[test]
    fn the_count_disc_is_as_wide_as_its_digits_but_never_narrower_than_it_is_tall() {
        let cases = [
            (1, 16.0),
            (3, 16.0),
            (9, 16.0),
            (10, 22.0),
            (99, 22.0),
            (100, 29.0),
            (999, 29.0),
            (1000, 36.0),
        ];
        for (count, width) in cases {
            assert_eq!(disc_width(count), width, "{count}");
        }
    }

    #[test]
    fn the_disc_centre_sits_beside_the_pointer_past_the_icons_behind_the_front_one() {
        let cases = [
            (1, 0, (51.0, 22.0)),
            (3, 1, (51.0, 22.0)),
            (3, 2, (57.0, 22.0)),
            (3, 3, (63.0, 22.0)),
            (12, 3, (66.0, 22.0)),
            (150, 3, (69.5, 22.0)),
        ];
        for (count, depth, centre) in cases {
            assert_eq!(disc_centre(count, depth), centre, "{count} {depth}");
        }
    }

    #[test]
    fn the_centre_is_the_middle_of_the_disc_the_stack_draws() {
        for count in [3, 10, 100, 1000] {
            for depth in 1..=3 {
                let (across, down) = disc_centre(count, depth);
                let left = BESIDE_POINTER + (depth - 1) as f32 * STEP + COUNT_LEFT;
                assert_eq!(across - left, disc_width(count) / 2.0, "{count} {depth}");
                assert_eq!(down - BESIDE_POINTER, CHIP_COUNT_HEIGHT / 2.0, "{count}");
            }
        }
    }
}
