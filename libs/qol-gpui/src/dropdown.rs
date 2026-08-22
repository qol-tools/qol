use std::{cell::RefCell, rc::Rc};

use gpui::*;

use crate::scroll_list::{wheel_rows, ScrollList};

pub const DROPDOWN_MAX_VISIBLE: usize = 10;
pub const ROW_H: f32 = 26.0;
const MENU_ID: &str = "dropdown-menu";

#[derive(Clone, Copy, Debug)]
pub struct DropdownStyle {
    pub bg: u32,
    pub bg_selected: u32,
    pub border: u32,
    pub text: u32,
    pub text_selected: u32,
    pub accent: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DropdownItem {
    pub label: String,
    pub accent: Option<u32>,
}

impl DropdownItem {
    pub fn plain(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            accent: None,
        }
    }
}

#[derive(Debug)]
pub struct Dropdown {
    list: Rc<RefCell<ScrollList>>,
    count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropdownEvent {
    Moved,
    Pick(usize),
    Close,
}

impl Dropdown {
    pub fn open(count: usize, selected: usize) -> Self {
        let mut list = ScrollList::new(DROPDOWN_MAX_VISIBLE);
        list.selected = selected;
        list.sync(count);
        Self {
            list: Rc::new(RefCell::new(list)),
            count,
        }
    }

    pub fn move_up(&mut self) {
        let mut list = self.list.borrow_mut();
        list.move_up();
        list.sync(self.count);
    }

    pub fn move_down(&mut self) {
        let mut list = self.list.borrow_mut();
        list.move_down(self.count);
        list.sync(self.count);
    }

    pub fn selected(&self) -> usize {
        self.list.borrow().selected
    }

    pub fn handle_key(&mut self, key: &str) -> Option<DropdownEvent> {
        match key {
            "up" => {
                self.move_up();
                Some(DropdownEvent::Moved)
            }
            "down" => {
                self.move_down();
                Some(DropdownEvent::Moved)
            }
            "enter" | "return" | "space" => Some(DropdownEvent::Pick(self.selected())),
            "escape" | "left" => Some(DropdownEvent::Close),
            _ => None,
        }
    }

    pub fn render(&self, labels: &[String], style: DropdownStyle) -> impl IntoElement {
        let items = labels
            .iter()
            .cloned()
            .map(DropdownItem::plain)
            .collect::<Vec<_>>();
        self.render_items_with_click(&items, style, None)
    }

    pub fn render_items(&self, items: &[DropdownItem], style: DropdownStyle) -> impl IntoElement {
        self.render_items_with_click(items, style, None)
    }

    pub fn render_clickable(
        &self,
        id: impl Into<SharedString>,
        labels: &[String],
        style: DropdownStyle,
        on_click: impl Fn(usize, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let items = labels
            .iter()
            .cloned()
            .map(DropdownItem::plain)
            .collect::<Vec<_>>();
        self.render_items_clickable(id, &items, style, on_click)
    }

    pub fn render_items_clickable(
        &self,
        id: impl Into<SharedString>,
        items: &[DropdownItem],
        style: DropdownStyle,
        on_click: impl Fn(usize, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        self.render_items_with_click(items, style, Some((id.into(), Rc::new(on_click))))
    }

    fn render_items_with_click(
        &self,
        items: &[DropdownItem],
        style: DropdownStyle,
        on_click: Option<(SharedString, Rc<DropdownClick>)>,
    ) -> impl IntoElement {
        let list = self.list.borrow();
        let rows: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .skip(list.scroll_offset)
            .take(list.max_visible)
            .map(|(index, item)| {
                let selected = index == list.selected;
                let mut row = div()
                    .relative()
                    .h(px(ROW_H))
                    .px_2()
                    .py_1()
                    .text_size(px(qol_theme::TEXT_BODY))
                    .text_color(rgb(if selected {
                        style.text_selected
                    } else {
                        style.text
                    }))
                    .child(item.label.clone());
                if selected {
                    row = row.bg(rgb(style.bg_selected)).child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(px(3.0))
                            .bg(rgb(style.accent)),
                    );
                }
                if let Some(accent) = item.accent {
                    row = row.border_l_2().border_color(rgb(accent));
                }
                let Some((id, on_click)) = on_click.clone() else {
                    return row.into_any_element();
                };
                row.id((id, index))
                    .cursor(CursorStyle::PointingHand)
                    .on_click(move |event, window, cx| {
                        on_click(index, event, window, cx);
                    })
                    .into_any_element()
            })
            .collect();
        let menu_list = Rc::clone(&self.list);
        let count = items.len();
        drop(list);
        deferred(
            anchored().snap_to_window_with_margin(px(8.0)).child(
                div()
                    .id(MENU_ID)
                    .flex()
                    .flex_col()
                    .min_w(px(160.0))
                    .rounded(px(qol_theme::RADIUS_CARD))
                    .shadow(crate::kit::float_shadow(style.text))
                    .bg(rgb(style.bg))
                    .on_scroll_wheel(move |event, window, _cx| {
                        let mut list = menu_list.borrow_mut();
                        list.wheel_by(wheel_rows(&event.delta, ROW_H), count);
                        window.refresh();
                    })
                    .children(rows),
            ),
        )
    }
}

type DropdownClick = dyn Fn(usize, &ClickEvent, &mut Window, &mut App);

#[cfg(test)]
mod tests {
    use super::{Dropdown, DropdownEvent};

    #[test]
    fn open_seeds_selection_and_scrolls_it_into_view() {
        let cases = [
            (3usize, 1usize, 1usize, 0usize),
            (3, 9, 2, 0),
            (20, 12, 12, 3),
            (0, 0, 0, 0),
        ];
        for (count, seed, expected_selected, expected_offset) in cases {
            let dropdown = Dropdown::open(count, seed);
            assert_eq!(
                (dropdown.selected(), dropdown.list.borrow().scroll_offset),
                (expected_selected, expected_offset),
                "count {count} seed {seed}"
            );
        }
    }

    #[test]
    fn moves_clamp_to_bounds_and_follow_the_window() {
        let mut dropdown = Dropdown::open(20, 0);
        dropdown.move_up();
        assert_eq!(dropdown.selected(), 0, "up at top stays");
        for _ in 0..20 {
            dropdown.move_down();
        }
        assert_eq!(dropdown.selected(), 19, "down clamps to last");
        assert_eq!(
            dropdown.list.borrow().scroll_offset,
            10,
            "window follows selection"
        );
    }

    #[test]
    fn window_caps_the_visible_rows_at_the_maximum() {
        let dropdown = Dropdown::open(40, 0);
        assert_eq!(
            dropdown.list.borrow().visible_range(40).len(),
            super::DROPDOWN_MAX_VISIBLE,
            "a long menu never renders more than the max visible rows"
        );
        let dropdown = Dropdown::open(4, 0);
        assert_eq!(
            dropdown.list.borrow().visible_range(4).len(),
            4,
            "a short menu renders every row"
        );
    }

    #[test]
    fn key_handling_owns_navigation_pick_and_close() {
        let mut dropdown = Dropdown::open(3, 1);
        let cases = [
            ("down", Some(DropdownEvent::Moved), 2),
            ("up", Some(DropdownEvent::Moved), 1),
            ("enter", Some(DropdownEvent::Pick(1)), 1),
            ("return", Some(DropdownEvent::Pick(1)), 1),
            ("space", Some(DropdownEvent::Pick(1)), 1),
            ("escape", Some(DropdownEvent::Close), 1),
            ("left", Some(DropdownEvent::Close), 1),
            ("tab", None, 1),
        ];
        for (key, expected, selected) in cases {
            assert_eq!(dropdown.handle_key(key), expected, "key: {key}");
            assert_eq!(dropdown.selected(), selected, "key: {key}");
        }
    }

    #[test]
    fn wheel_offset_survives_into_visible_range() {
        let dropdown = Dropdown::open(40, 0);
        {
            let mut list = dropdown.list.borrow_mut();
            list.wheel_by(24, 40);
            assert_eq!(list.scroll_offset, 24, "wheel advances toward later items");
        }
        let visible = dropdown.list.borrow().visible_range(40);
        assert_eq!(visible, 24..34, "wheel-driven offset seeds the window");
    }
}
