use super::WindowDelegate;

pub(crate) enum GridDirection {
    Left,
    Right,
    Up,
    Down,
}

impl WindowDelegate {
    pub(crate) fn select_next(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.selected_index.unwrap_or(0);
        self.selected_index = Some((current + 1) % self.windows.len());
    }

    pub(crate) fn select_prev(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.selected_index.unwrap_or(0);
        self.selected_index = Some(if current == 0 {
            self.windows.len() - 1
        } else {
            current - 1
        });
    }

    pub(crate) fn select_left(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Left, columns);
    }

    pub(crate) fn select_right(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Right, columns);
    }

    pub(crate) fn select_up(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Up, columns);
    }

    pub(crate) fn select_down(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Down, columns);
    }

    fn move_in_grid(&mut self, direction: GridDirection, columns: usize) {
        let total = self.windows.len();
        if total == 0 {
            return;
        }

        let cols = columns.max(1).min(total);
        let rows = (total + cols - 1) / cols;
        let current = self
            .selected_index
            .unwrap_or(0)
            .min(total.saturating_sub(1));

        let row = current / cols;
        let col = current % cols;

        let row_bounds = |r: usize| {
            let start = r * cols;
            let end = ((r + 1) * cols).min(total);
            (start, end)
        };

        let next = match direction {
            GridDirection::Left => {
                let (row_start, _) = row_bounds(row);
                if current > row_start {
                    current - 1
                } else {
                    current
                }
            }
            GridDirection::Right => {
                let (_, row_end) = row_bounds(row);
                if current + 1 < row_end {
                    current + 1
                } else {
                    current
                }
            }
            GridDirection::Up => {
                if row == 0 {
                    current
                } else {
                    let (target_start, target_end) = row_bounds(row - 1);
                    target_start + col.min(target_end - target_start - 1)
                }
            }
            GridDirection::Down => {
                if row + 1 >= rows {
                    current
                } else {
                    let (target_start, target_end) = row_bounds(row + 1);
                    target_start + col.min(target_end - target_start - 1)
                }
            }
        };

        self.selected_index = Some(next);
    }
}
