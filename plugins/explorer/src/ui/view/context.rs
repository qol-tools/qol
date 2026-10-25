use gpui::{point, px, Context, MouseDownEvent, Pixels, Point};
use qol_gpui::theme::SPACE_GUTTER;

use super::menu::Menu;
use super::{ExplorerView, ICON};

impl ExplorerView {
    pub(super) fn context_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.browser.end_range();
        self.browser.close_path();
        let area = self.pressing.area().get();
        let row = area
            .contains(&event.position)
            .then(|| {
                let x = f32::from(event.position.x - area.origin.x);
                let y = f32::from(event.position.y - area.origin.y) + self.scroll_offset();
                self.layout()
                    .tiles(self.list_width)
                    .at(x, y, self.browser.row_count())
            })
            .flatten();
        match row {
            Some(row) => self.open_item_menu(row, event.position, cx),
            None => self.open_folder_menu(event.position, true, cx),
        }
    }

    pub(super) fn open_cursor_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let row = self.browser.cursor();
        let area = self.pressing.area().get();
        if self.browser.entry(row).is_none() {
            let at = area.origin + point(px(SPACE_GUTTER), px(SPACE_GUTTER));
            self.open_folder_menu(at, true, cx);
            return true;
        }
        let at = self.cursor_point();
        self.open_item_menu(row, at, cx);
        true
    }

    pub(super) fn cursor_point(&mut self) -> Point<Pixels> {
        let row = self.browser.cursor();
        let area = self.pressing.area().get();
        self.reveal_cursor();
        let rect = self.layout().tiles(self.list_width).rect(row);
        let top = rect.top - self.scroll_offset() + (rect.bottom - rect.top) / 2.0;
        area.origin + point(px(rect.left + SPACE_GUTTER + ICON), px(top))
    }

    fn open_item_menu(&mut self, row: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        let scroll = self.scroll_offset();
        if !self.browser.lift(row, false, scroll) {
            return;
        }
        self.renaming = None;
        self.menu = Some(Menu::item(
            self.browser.holds(),
            self.browser.operands().len() == 1,
            self.browser.cursor_file().is_some(),
            at,
        ));
        cx.notify();
    }
}
