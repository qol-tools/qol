pub fn clamp_into_view(
    selected: &mut usize,
    scroll_offset: &mut usize,
    count: usize,
    max_visible: usize,
) {
    if count == 0 {
        *selected = 0;
        *scroll_offset = 0;
        return;
    }

    *selected = (*selected).min(count - 1);
    let max_offset = count.saturating_sub(max_visible);
    *scroll_offset = (*scroll_offset).min(max_offset);

    if *selected < *scroll_offset {
        *scroll_offset = *selected;
        return;
    }

    let bottom = *scroll_offset + max_visible.saturating_sub(1);
    if *selected > bottom {
        *scroll_offset = *selected + 1 - max_visible;
    }
}

#[derive(Debug, Clone)]
pub struct ScrollList {
    pub selected: usize,
    pub scroll_offset: usize,
    pub max_visible: usize,
}

impl ScrollList {
    pub fn new(max_visible: usize) -> Self {
        Self {
            selected: 0,
            scroll_offset: 0,
            max_visible: max_visible.max(1),
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self, count: usize) {
        if count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
    }

    pub fn reset(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
    }

    pub fn sync(&mut self, count: usize) {
        clamp_into_view(
            &mut self.selected,
            &mut self.scroll_offset,
            count,
            self.max_visible,
        );
    }

    pub fn visible_range(&self, count: usize) -> std::ops::Range<usize> {
        let start = self.scroll_offset;
        let end = (start + self.max_visible).min(count);
        start..end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_selected_inside_the_window() {
        let cases = [
            (0usize, 0usize, 0usize, 5usize),
            (10, 0, 20, 5),
            (3, 0, 20, 5),
            (19, 0, 20, 5),
            (0, 10, 20, 5),
            (2, 0, 3, 5),
        ];
        for (sel, scroll, count, vis) in cases {
            let (mut s, mut off) = (sel, scroll);
            clamp_into_view(&mut s, &mut off, count, vis);
            if count == 0 {
                assert_eq!((s, off), (0, 0), "empty resets");
                continue;
            }
            let label = format!("sel={sel} scroll={scroll} count={count} vis={vis}");
            assert!(s < count, "selected in bounds: {label}");
            assert!(s >= off, "selected at/after window start: {label}");
            assert!(s < off + vis, "selected before window end: {label}");
        }
    }

    #[test]
    fn navigating_down_scrolls_the_window_to_follow() {
        let mut list = ScrollList::new(5);
        let count = 20;
        for _ in 0..12 {
            list.move_down(count);
            list.sync(count);
        }
        assert_eq!(list.selected, 12, "selection advanced");
        assert!(list.visible_range(count).contains(&list.selected), "selection visible");
    }

    #[test]
    fn move_up_saturates_at_zero() {
        let mut list = ScrollList::new(5);
        list.move_up();
        list.move_up();
        assert_eq!(list.selected, 0);
    }
}
