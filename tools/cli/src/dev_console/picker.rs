pub(super) const FILTER_PANEL_MIN_WIDTH: u16 = 32;
pub(super) const FILTER_PANEL_MAX_WIDTH: u16 = 78;
const FILTER_BRICK_GAP: usize = 1;
pub(super) const FILTER_BRICK_CHROME: usize = 4;

#[derive(Debug, PartialEq, Eq, Clone)]
pub(super) struct PickerBrick {
    pub(super) index: usize,
    pub(super) row: usize,
    pub(super) x: usize,
    pub(super) width: usize,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum PickerMove {
    Left,
    Right,
    Up,
    Down,
}

pub(super) fn picker_brick_layout<T>(
    items: &[T],
    width: usize,
    mut item_width: impl FnMut(&T, usize) -> usize,
) -> Vec<PickerBrick> {
    let width = width.max(1);
    let mut row = 0;
    let mut x = 0;
    let mut layout = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let brick_width = item_width(item, width);
        if x > 0 && x + FILTER_BRICK_GAP + brick_width > width {
            row += 1;
            x = 0;
        }
        let gap = if x == 0 { 0 } else { FILTER_BRICK_GAP };
        let brick_x = x + gap;
        layout.push(PickerBrick {
            index,
            row,
            x: brick_x,
            width: brick_width,
        });
        x = brick_x + brick_width;
    }
    layout
}

pub(super) fn move_picker_selection(
    selected: &mut usize,
    item_count: usize,
    direction: PickerMove,
    layout: &[PickerBrick],
) {
    if item_count == 0 {
        *selected = 0;
        return;
    }
    if matches!(direction, PickerMove::Left | PickerMove::Right) {
        let len = item_count as isize;
        let delta = if matches!(direction, PickerMove::Left) {
            -1
        } else {
            1
        };
        *selected = (*selected as isize + delta).rem_euclid(len) as usize;
        return;
    }
    let Some(current) = layout.iter().find(|brick| brick.index == *selected) else {
        *selected = 0;
        return;
    };
    let Some(max_row) = layout.iter().map(|brick| brick.row).max() else {
        return;
    };
    let target_row = match direction {
        PickerMove::Up if current.row == 0 => max_row,
        PickerMove::Up => current.row - 1,
        PickerMove::Down if current.row == max_row => 0,
        PickerMove::Down => current.row + 1,
        PickerMove::Left | PickerMove::Right => current.row,
    };
    let center = brick_center(current);
    let Some(target) = layout
        .iter()
        .filter(|brick| brick.row == target_row)
        .min_by_key(|brick| {
            (
                brick_center(brick).abs_diff(center),
                brick.index.abs_diff(current.index),
            )
        })
    else {
        return;
    };
    *selected = target.index;
}

pub(super) fn brick_center(brick: &PickerBrick) -> usize {
    brick.x.saturating_mul(2) + brick.width
}

pub(super) fn filter_text(raw: &str, max_width: usize) -> String {
    raw.chars().take(max_width.max(1)).collect()
}

pub(super) fn filter_text_width(row_width: usize) -> usize {
    row_width.saturating_sub(FILTER_BRICK_CHROME).max(1)
}

pub(super) fn default_filter_layout_width() -> usize {
    FILTER_PANEL_MAX_WIDTH.saturating_sub(2) as usize
}
