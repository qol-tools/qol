use std::path::Path;

use gpui::{
    div, px, App, AppContext, Context, Entity, IntoElement, ParentElement, Pixels, Point, Render,
    Styled, WeakEntity, Window,
};
use qol_gpui::kit::kit;

use super::pictogram::lifted_pictogram;
use super::ExplorerView;
use crate::listing::Entry;

const BESIDE_POINTER: f32 = 14.0;
const ICON: f32 = 40.0;
const STEP: f32 = 6.0;
const STACKED: usize = 3;
const COUNT_FROM: usize = 3;
const COUNT_LEFT: f32 = 29.0;
const COUNT_RISE: f32 = 4.0;

pub(super) struct DraggedPaths {
    count: usize,
    stack: Vec<Entry>,
    grab: Point<Pixels>,
}

impl Render for DraggedPaths {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let kit = kit();
        let behind = self.stack.len().saturating_sub(1) as f32 * STEP;
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
                .collect();
            (paths, stack)
        })
        .unwrap_or_else(|| (vec![path.to_path_buf()], Vec::new()));
    let count = paths.len();
    window.offer_dragged_paths(paths);
    cx.new(|_| DraggedPaths { count, stack, grab })
}
