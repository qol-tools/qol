use gpui::ScrollHandle;

/// Keeps a gpui-native scrolling container following a keyboard selection.
///
/// A container that scrolls itself with `overflow_y_scroll` has no idea which
/// child is selected, so the selection walks off the bottom while the viewport
/// stays put. This reissues a scroll only when the selection actually moves,
/// which leaves wheel and trackpad scrolling alone in between.
#[derive(Debug, Clone, Default)]
pub struct SelectionScroll {
    handle: ScrollHandle,
    followed: std::cell::Cell<Option<usize>>,
}

impl SelectionScroll {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&self) -> &ScrollHandle {
        &self.handle
    }

    pub fn follow(&self, selected: Option<usize>) {
        if self.followed.get() == selected {
            return;
        }
        self.followed.set(selected);
        if let Some(index) = selected {
            self.handle.scroll_to_item(index);
        }
    }

    pub fn rewind(&self) {
        self.followed.set(None);
        self.handle.set_offset(gpui::Point::default());
    }
}

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

pub fn shift_window(scroll_offset: usize, steps: i32, count: usize, max_visible: usize) -> usize {
    let max_offset = count.saturating_sub(max_visible);
    let target = scroll_offset as isize + steps as isize;
    target.clamp(0, max_offset as isize) as usize
}

pub fn accumulate_steps(remainder: &mut f32, increments: f32) -> i32 {
    if !increments.is_finite() || increments == 0.0 {
        return 0;
    }
    if *remainder != 0.0 && remainder.signum() != increments.signum() {
        *remainder = 0.0;
    }
    *remainder += increments;
    let steps = remainder.trunc() as i32;
    if steps == 0 {
        return 0;
    }
    if !(-1..=1).contains(&steps) {
        *remainder = 0.0;
        return steps.signum();
    }
    *remainder -= steps as f32;
    steps
}

#[derive(Debug, Clone)]
pub struct ScrollList {
    pub selected: usize,
    pub scroll_offset: usize,
    pub max_visible: usize,
    scroll_accum: f32,
}

impl ScrollList {
    pub fn new(max_visible: usize) -> Self {
        Self {
            selected: 0,
            scroll_offset: 0,
            max_visible: max_visible.max(1),
            scroll_accum: 0.0,
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

    pub fn scroll_steps(&mut self, steps: i32, count: usize) {
        self.scroll_offset = shift_window(self.scroll_offset, steps, count, self.max_visible);
    }

    pub fn wheel_by(&mut self, rows: isize, total: usize) {
        let max_offset = total.saturating_sub(self.max_visible);
        let target = self.scroll_offset as isize + rows;
        self.scroll_offset = target.clamp(0, max_offset as isize) as usize;
    }

    pub fn scroll_by(&mut self, increments: f32, count: usize) {
        let steps = accumulate_steps(&mut self.scroll_accum, increments);
        self.scroll_steps(steps, count);
    }

    pub fn reset(&mut self) {
        self.selected = 0;
        self.scroll_offset = 0;
        self.scroll_accum = 0.0;
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

pub fn wheel_rows(delta: &gpui::ScrollDelta, row_height: f32) -> isize {
    let lines = match delta {
        gpui::ScrollDelta::Lines(lines) => lines.y,
        gpui::ScrollDelta::Pixels(pixels) => pixels.y.to_f64() as f32 / row_height,
    };
    -lines.round() as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

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
    fn bottom_follow_scrolls_offset_to_reveal_selection() {
        let (mut selected, mut offset) = (7usize, 0usize);
        clamp_into_view(&mut selected, &mut offset, 20, 5);
        assert_eq!(
            (selected, offset),
            (7, 3),
            "offset follows the selection past the window bottom"
        );
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
        assert!(
            list.visible_range(count).contains(&list.selected),
            "selection visible"
        );
    }

    #[test]
    fn move_up_saturates_at_zero() {
        let mut list = ScrollList::new(5);
        list.move_up();
        list.move_up();
        assert_eq!(list.selected, 0);
    }

    #[test]
    fn selection_scroll_reissues_only_when_the_selection_moves() {
        let scroll = SelectionScroll::new();

        scroll.follow(Some(4));
        assert_eq!(scroll.followed.get(), Some(4));

        scroll.follow(Some(4));
        assert_eq!(
            scroll.followed.get(),
            Some(4),
            "a repeat render is not a move"
        );

        scroll.follow(Some(9));
        assert_eq!(scroll.followed.get(), Some(9));

        scroll.follow(None);
        assert_eq!(
            scroll.followed.get(),
            None,
            "an emptied list clears the target"
        );
    }

    #[test]
    fn rewinding_lets_the_same_index_be_followed_again() {
        let scroll = SelectionScroll::new();
        scroll.follow(Some(3));
        scroll.rewind();
        assert_eq!(scroll.followed.get(), None);

        scroll.follow(Some(3));
        assert_eq!(scroll.followed.get(), Some(3));
    }

    #[test]
    fn shift_window_clamps_at_both_ends() {
        let cases = [
            (0usize, 1i32, 12usize, 5usize, 1usize),
            (7, 1, 12, 5, 7),
            (12, -1, 12, 5, 7),
            (0, -3, 12, 5, 0),
            (1, 10, 12, 5, 7),
            (3, 20, 3, 5, 0),
            (2, -1, 6, 3, 1),
        ];
        for (offset, steps, count, vis, expected) in cases {
            assert_eq!(
                shift_window(offset, steps, count, vis),
                expected,
                "offset={offset} steps={steps} count={count} vis={vis}"
            );
        }
    }

    #[test]
    fn shift_window_never_exceeds_the_window_end() {
        for count in 0usize..=30 {
            for max_visible in 1usize..=8 {
                let max_offset = count.saturating_sub(max_visible);
                for initial in 0usize..=max_offset {
                    for steps in -5i32..=5 {
                        let shifted = shift_window(initial, steps, count, max_visible);
                        assert!(
                            shifted <= max_offset,
                            "count={count} vis={max_visible} off={initial} steps={steps}: got {shifted}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn wheel_scroll_keeps_the_selection_put() {
        let mut list = ScrollList::new(5);
        list.move_down(12);
        list.sync(12);
        let before = list.selected;
        for steps in [-4, -1, 1, 3, 9] {
            list.scroll_steps(steps, 12);
            assert_eq!(list.selected, before, "steps={steps} moved the selection");
        }
    }

    #[test]
    fn wheel_scroll_accumulates_fractional_deltas() {
        let mut list = ScrollList::new(5);
        list.scroll_by(0.4, 12);
        assert_eq!(list.scroll_offset, 0, "sub-notch stays put");
        list.scroll_by(0.4, 12);
        list.scroll_by(0.4, 12);
        assert_eq!(list.scroll_offset, 1, "three 0.4 increments cross one item");
    }

    #[test]
    fn wheel_scroll_respects_the_direction_switch() {
        let mut list = ScrollList::new(5);
        list.scroll_by(1.0, 12);
        list.scroll_by(1.0, 12);
        let advanced = list.scroll_offset;
        list.scroll_by(-4.0, 12);
        assert!(
            list.scroll_offset < advanced,
            "opposite-wheel must retreat the window"
        );
    }

    #[test]
    fn wheel_by_clamps_at_the_window_start() {
        let mut list = ScrollList::new(5);
        list.scroll_offset = 3;
        list.wheel_by(-10, 12);
        assert_eq!(list.scroll_offset, 0, "cannot scroll above the first row");
    }

    #[test]
    fn wheel_by_clamps_at_the_window_end() {
        let mut list = ScrollList::new(5);
        list.scroll_offset = 3;
        list.wheel_by(10, 12);
        assert_eq!(
            list.scroll_offset, 7,
            "cannot scroll past the last visible row"
        );
    }

    #[test]
    fn wheel_by_advances_toward_later_items() {
        let mut list = ScrollList::new(5);
        list.wheel_by(3, 20);
        list.wheel_by(2, 20);
        assert_eq!(list.scroll_offset, 5, "positive rows reveal later items");
    }

    #[test]
    fn wheel_rows_converts_line_and_pixel_deltas() {
        let toward_user = gpui::ScrollDelta::Lines(gpui::point(0.0, -3.0));
        assert_eq!(
            wheel_rows(&toward_user, 44.0),
            3,
            "wheel toward the user reveals later items"
        );

        let pixel_delta = gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-110.0)));
        assert_eq!(
            wheel_rows(&pixel_delta, 44.0),
            3,
            "pixels divided by row height"
        );

        let away = gpui::ScrollDelta::Lines(gpui::point(0.0, 2.0));
        assert_eq!(wheel_rows(&away, 44.0), -2, "away from the user retreats");
    }
}
