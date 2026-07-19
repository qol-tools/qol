use std::rc::Rc;

use gpui::*;

use crate::scroll_list::ScrollList;

pub const DROPDOWN_MAX_VISIBLE: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct DropdownStyle {
    pub bg: u32,
    pub bg_selected: u32,
    pub border: u32,
    pub text: u32,
    pub text_selected: u32,
}

#[derive(Debug)]
pub struct Dropdown {
    list: ScrollList,
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
        Self { list, count }
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
        self.list.sync(self.count);
    }

    pub fn move_down(&mut self) {
        self.list.move_down(self.count);
        self.list.sync(self.count);
    }

    pub fn selected(&self) -> usize {
        self.list.selected
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
        self.render_with_click(labels, style, None)
    }

    pub fn render_clickable(
        &self,
        id: impl Into<SharedString>,
        labels: &[String],
        style: DropdownStyle,
        on_click: impl Fn(usize, &ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        self.render_with_click(labels, style, Some((id.into(), Rc::new(on_click))))
    }

    fn render_with_click(
        &self,
        labels: &[String],
        style: DropdownStyle,
        on_click: Option<(SharedString, Rc<DropdownClick>)>,
    ) -> impl IntoElement {
        let rows: Vec<AnyElement> = labels
            .iter()
            .enumerate()
            .skip(self.list.scroll_offset)
            .take(self.list.max_visible)
            .map(|(index, label)| {
                let selected = index == self.list.selected;
                let mut row = div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(rgb(if selected {
                        style.text_selected
                    } else {
                        style.text
                    }))
                    .child(label.clone());
                if selected {
                    row = row.bg(rgb(style.bg_selected));
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
        deferred(
            anchored().snap_to_window_with_margin(px(8.0)).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(160.0))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(style.border))
                    .bg(rgb(style.bg))
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
            (20, 12, 12, 5),
            (0, 0, 0, 0),
        ];
        for (count, seed, expected_selected, expected_offset) in cases {
            let dropdown = Dropdown::open(count, seed);
            assert_eq!(
                (dropdown.selected(), dropdown.list.scroll_offset),
                (expected_selected, expected_offset),
                "count {count} seed {seed}"
            );
        }
    }

    #[test]
    fn moves_clamp_to_bounds_and_follow_the_window() {
        let mut dropdown = Dropdown::open(10, 0);
        dropdown.move_up();
        assert_eq!(dropdown.selected(), 0, "up at top stays");
        for _ in 0..20 {
            dropdown.move_down();
        }
        assert_eq!(dropdown.selected(), 9, "down clamps to last");
        assert_eq!(dropdown.list.scroll_offset, 2, "window follows selection");
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
}
