use std::path::Path;

use gpui::{
    div, point, px, App, AppContext, Context, DragImage, Entity, IntoElement, ParentElement,
    Pixels, Point, Render, Styled, WeakEntity, Window,
};
use qol_gpui::kit::{kit, CHIP_COUNT_HEIGHT};
use qol_gpui::theme::{count_disc, font_ui, TextStyle, SPACE_TIGHT};

use super::pictogram::{lifted_pictogram, lifted_source, Lifted};
use super::ExplorerView;
use crate::listing::Entry;

const BESIDE_POINTER: f32 = 14.0;
const ICON: f32 = 40.0;
const STEP: f32 = 6.0;
const STACKED: usize = 3;
const COUNT_FROM: usize = 3;
const COUNT_LEFT: f32 = 29.0;
const COUNT_RISE: f32 = 4.0;
const DIGIT_WIDTH: f32 = 7.0;

pub(super) struct DraggedPaths {
    count: usize,
    stack: Vec<Entry>,
    grab: Point<Pixels>,
}

impl Render for DraggedPaths {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let kit = kit();
        let behind = behind(self.stack.len());
        let icons = self.stack.iter().enumerate().rev().map(|(depth, entry)| {
            let offset = depth as f32 * STEP;
            div()
                .absolute()
                .left(px(offset))
                .top(px(COUNT_RISE + behind - offset))
                .child(lifted_pictogram(entry, ICON))
        });
        let count = (self.count >= COUNT_FROM).then(|| {
            div()
                .absolute()
                .left(px(behind + COUNT_LEFT))
                .top_0()
                .child(kit.count_disc(self.count, kit.grounds.pane))
        });
        div()
            .pl(self.grab.x + px(BESIDE_POINTER))
            .pt(self.grab.y + px(BESIDE_POINTER))
            .child(
                div()
                    .relative()
                    .w(px(ICON + behind + COUNT_LEFT))
                    .h(px(ICON + behind + COUNT_RISE))
                    .children(icons)
                    .children(count),
            )
    }
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
            let browser = &view.read(cx).browser;
            let paths = browser.drag_paths(path);
            let entries = browser.listing().entries();
            let stack = std::iter::once(path)
                .chain(
                    paths
                        .iter()
                        .map(|path| path.as_path())
                        .filter(|&other| other != path),
                )
                .filter_map(|path| entries.iter().find(|entry| entry.path == path).cloned())
                .take(STACKED)
                .collect::<Vec<_>>();
            (paths, stack)
        })
        .unwrap_or_else(|| (vec![path.to_path_buf()], Vec::new()));
    let count = paths.len();
    let image = drag_image(&stack, count, window.scale_factor());
    window.offer_dragged_paths(paths, image);
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
    let width = (text.len() as f32 * DIGIT_WIDTH + SPACE_TIGHT * 2.0).max(CHIP_COUNT_HEIGHT);
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
