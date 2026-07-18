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

    pub fn render(&self, labels: &[String], style: DropdownStyle) -> impl IntoElement {
        let rows: Vec<Div> = labels
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
                row
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

#[cfg(test)]
mod tests {
    use super::Dropdown;

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
}
