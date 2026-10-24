use std::path::Path;

use gpui::{
    div, px, App, AppContext, Context, Entity, IntoElement, ParentElement, Pixels, Point, Render,
    Styled, WeakEntity, Window,
};
use qol_gpui::kit::kit;

use super::ExplorerView;

const BESIDE_POINTER: f32 = 14.0;

pub(super) struct DraggedPaths {
    count: usize,
    grab: Point<Pixels>,
}

impl Render for DraggedPaths {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let kit = kit();
        div()
            .pl(self.grab.x + px(BESIDE_POINTER))
            .pt(self.grab.y + px(BESIDE_POINTER))
            .child(kit.count_disc(self.count, kit.grounds.pane))
    }
}

pub(super) fn start(
    view: &WeakEntity<ExplorerView>,
    path: &Path,
    grab: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DraggedPaths> {
    let paths = view
        .upgrade()
        .map(|view| view.read(cx).browser.drag_paths(path))
        .unwrap_or_else(|| vec![path.to_path_buf()]);
    let count = paths.len();
    window.offer_dragged_paths(paths);
    cx.new(|_| DraggedPaths { count, grab })
}
