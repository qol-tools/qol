#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub fn spanning(a: (f32, f32), b: (f32, f32)) -> Self {
        Self {
            left: a.0.min(b.0),
            top: a.1.min(b.1),
            right: a.0.max(b.0),
            bottom: a.1.max(b.1),
        }
    }

    pub fn touches(self, tile: Rect) -> bool {
        self.left < tile.right
            && self.right >= tile.left
            && self.top < tile.bottom
            && self.bottom >= tile.top
    }

    pub fn within(self, area: Rect) -> Option<Rect> {
        let inside = Rect {
            left: self.left.max(area.left),
            top: self.top.max(area.top),
            right: self.right.min(area.right),
            bottom: self.bottom.min(area.bottom),
        };
        (inside.left <= inside.right && inside.top <= inside.bottom).then_some(inside)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiles {
    pub columns: usize,
    pub pitch: f32,
    pub size: (f32, f32),
    pub origin: (f32, f32),
    pub gap: f32,
}

impl Tiles {
    pub fn rect(self, index: usize) -> Rect {
        let (line, column) = (index / self.columns, index % self.columns);
        let left = self.origin.0 + column as f32 * (self.size.0 + self.gap);
        let top = self.origin.1 + line as f32 * self.pitch;
        Rect {
            left,
            top,
            right: left + self.size.0,
            bottom: top + self.size.1,
        }
    }

    pub fn at(self, x: f32, y: f32, total: usize) -> Option<usize> {
        let (across, down) = (x - self.origin.0, y - self.origin.1);
        if across < 0.0 || down < 0.0 {
            return None;
        }
        let line = (down / self.pitch).floor() as usize;
        let inner = down - line as f32 * self.pitch;
        let stride = self.size.0 + self.gap;
        let column = (across / stride).floor() as usize;
        let beside = across - column as f32 * stride;
        let index = line * self.columns + column;
        let hit = inner < self.size.1 && beside < self.size.0 && column < self.columns;
        (hit && index < total).then_some(index)
    }

    pub fn touched(self, boxed: Rect, total: usize) -> Vec<usize> {
        let lines = total.div_ceil(self.columns);
        let below = boxed.bottom - self.origin.1;
        if lines == 0 || below < 0.0 {
            return Vec::new();
        }
        let first = ((boxed.top - self.origin.1 - self.size.1) / self.pitch).floor() + 1.0;
        let first = first.max(0.0) as usize;
        let last = ((below / self.pitch).floor() as usize).min(lines - 1);
        (first..=last)
            .flat_map(|line| (0..self.columns).map(move |column| line * self.columns + column))
            .filter(|&index| index < total && boxed.touches(self.rect(index)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Rect, Tiles};

    const LIST: Tiles = Tiles {
        columns: 1,
        pitch: 36.0,
        size: (800.0, 36.0),
        origin: (0.0, 0.0),
        gap: 0.0,
    };

    const GRID: Tiles = Tiles {
        columns: 4,
        pitch: 104.0,
        size: (120.0, 96.0),
        origin: (20.0, 8.0),
        gap: 14.0,
    };

    fn spanning(a: (f32, f32), b: (f32, f32)) -> Rect {
        Rect::spanning(a, b)
    }

    #[test]
    fn spanning_orders_the_corners_whichever_way_the_pointer_went() {
        let want = Rect {
            left: 1.0,
            top: 2.0,
            right: 5.0,
            bottom: 9.0,
        };
        let cases = [
            ((1.0, 2.0), (5.0, 9.0)),
            ((5.0, 9.0), (1.0, 2.0)),
            ((5.0, 2.0), (1.0, 9.0)),
            ((1.0, 9.0), (5.0, 2.0)),
        ];
        for (a, b) in cases {
            assert_eq!(spanning(a, b), want, "{a:?} {b:?}");
        }
    }

    #[test]
    fn a_box_touches_a_tile_when_they_overlap_and_a_flat_box_still_does() {
        let tile = Rect {
            left: 10.0,
            top: 10.0,
            right: 20.0,
            bottom: 20.0,
        };
        let cases = [
            (spanning((12.0, 12.0), (15.0, 15.0)), true),
            (spanning((0.0, 0.0), (30.0, 30.0)), true),
            (spanning((15.0, 0.0), (15.0, 30.0)), true),
            (spanning((0.0, 15.0), (30.0, 15.0)), true),
            (spanning((15.0, 15.0), (15.0, 15.0)), true),
            (spanning((0.0, 0.0), (10.0, 10.0)), true),
            (spanning((0.0, 0.0), (9.9, 30.0)), false),
            (spanning((20.0, 0.0), (30.0, 30.0)), false),
            (spanning((0.0, 20.0), (30.0, 30.0)), false),
            (spanning((0.0, 0.0), (30.0, 9.9)), false),
            (spanning((25.0, 25.0), (40.0, 40.0)), false),
        ];
        for (boxed, touches) in cases {
            assert_eq!(boxed.touches(tile), touches, "{boxed:?}");
        }
    }

    #[test]
    fn within_clips_a_box_to_the_area_and_drops_one_wholly_outside() {
        let area = Rect {
            left: 0.0,
            top: 0.0,
            right: 100.0,
            bottom: 50.0,
        };
        let cases = [
            (
                spanning((-10.0, -10.0), (30.0, 20.0)),
                Some(spanning((0.0, 0.0), (30.0, 20.0))),
            ),
            (
                spanning((50.0, 40.0), (200.0, 90.0)),
                Some(spanning((50.0, 40.0), (100.0, 50.0))),
            ),
            (
                spanning((-5.0, -5.0), (500.0, 500.0)),
                Some(spanning((0.0, 0.0), (100.0, 50.0))),
            ),
            (spanning((110.0, 0.0), (200.0, 10.0)), None),
            (spanning((0.0, 60.0), (10.0, 80.0)), None),
        ];
        for (boxed, clipped) in cases {
            assert_eq!(boxed.within(area), clipped, "{boxed:?}");
        }
    }

    #[test]
    fn tile_rectangles_follow_the_line_and_column_of_the_index() {
        let cases = [
            (LIST, 0, (0.0, 0.0, 800.0, 36.0)),
            (LIST, 3, (0.0, 108.0, 800.0, 144.0)),
            (GRID, 0, (20.0, 8.0, 140.0, 104.0)),
            (GRID, 1, (154.0, 8.0, 274.0, 104.0)),
            (GRID, 3, (422.0, 8.0, 542.0, 104.0)),
            (GRID, 4, (20.0, 112.0, 140.0, 208.0)),
            (GRID, 9, (154.0, 216.0, 274.0, 312.0)),
        ];
        for (tiles, index, (left, top, right, bottom)) in cases {
            assert_eq!(
                tiles.rect(index),
                Rect {
                    left,
                    top,
                    right,
                    bottom
                },
                "{tiles:?} {index}"
            );
        }
    }

    #[test]
    fn a_point_finds_the_tile_under_it_and_nothing_in_the_gaps_or_past_the_end() {
        let cases = [
            (LIST, 5.0, 5.0, 10, Some(0)),
            (LIST, 799.0, 35.9, 10, Some(0)),
            (LIST, 800.0, 10.0, 10, None),
            (LIST, -1.0, 10.0, 10, None),
            (LIST, 5.0, -1.0, 10, None),
            (LIST, 5.0, 36.0, 10, Some(1)),
            (LIST, 5.0, 359.9, 10, Some(9)),
            (LIST, 5.0, 360.0, 10, None),
            (LIST, 5.0, 5.0, 0, None),
            (GRID, 19.9, 20.0, 12, None),
            (GRID, 20.0, 20.0, 12, Some(0)),
            (GRID, 139.9, 103.9, 12, Some(0)),
            (GRID, 140.0, 50.0, 12, None),
            (GRID, 153.9, 50.0, 12, None),
            (GRID, 154.0, 50.0, 12, Some(1)),
            (GRID, 300.0, 7.9, 12, None),
            (GRID, 300.0, 104.0, 12, None),
            (GRID, 300.0, 112.0, 12, Some(6)),
            (GRID, 430.0, 112.0, 12, Some(7)),
            (GRID, 560.0, 112.0, 12, None),
            (GRID, 300.0, 216.0, 12, Some(10)),
            (GRID, 430.0, 216.0, 11, None),
            (GRID, 430.0, 216.0, 12, Some(11)),
        ];
        for (tiles, x, y, total, index) in cases {
            assert_eq!(tiles.at(x, y, total), index, "{tiles:?} {x} {y} {total}");
        }
    }

    #[test]
    fn at_and_rect_agree_on_every_tile_corner_and_centre() {
        for tiles in [LIST, GRID] {
            for index in 0..23 {
                let rect = tiles.rect(index);
                let probes = [
                    (rect.left, rect.top),
                    (
                        (rect.left + rect.right) / 2.0,
                        (rect.top + rect.bottom) / 2.0,
                    ),
                    (rect.right - 0.01, rect.bottom - 0.01),
                ];
                for (x, y) in probes {
                    assert_eq!(tiles.at(x, y, 23), Some(index), "{tiles:?} {index}");
                }
            }
        }
    }

    #[test]
    fn a_box_in_the_list_picks_every_row_it_reaches() {
        let cases = [
            (spanning((10.0, 40.0), (20.0, 50.0)), 10, vec![1]),
            (spanning((10.0, 30.0), (20.0, 40.0)), 10, vec![0, 1]),
            (spanning((10.0, 36.0), (20.0, 36.0)), 10, vec![1]),
            (spanning((10.0, 35.9), (20.0, 35.9)), 10, vec![0]),
            (spanning((-50.0, -50.0), (-40.0, -40.0)), 10, vec![]),
            (spanning((-50.0, -50.0), (5.0, 0.0)), 10, vec![0]),
            (spanning((0.0, 0.0), (5.0, 360.0)), 10, (0..10).collect()),
            (spanning((0.0, 0.0), (5.0, 9000.0)), 10, (0..10).collect()),
            (
                spanning((0.0, 100.0), (5.0, 9000.0)),
                10,
                vec![2, 3, 4, 5, 6, 7, 8, 9],
            ),
            (spanning((0.0, 400.0), (5.0, 500.0)), 10, vec![]),
            (spanning((0.0, 0.0), (5.0, 500.0)), 0, vec![]),
            (spanning((799.0, 0.0), (900.0, 10.0)), 10, vec![0]),
            (spanning((800.0, 0.0), (900.0, 10.0)), 10, vec![]),
        ];
        for (boxed, total, rows) in cases {
            assert_eq!(LIST.touched(boxed, total), rows, "{boxed:?} {total}");
        }
    }

    #[test]
    fn a_box_in_the_grid_picks_the_cells_it_reaches_across_and_down() {
        let cases = [
            (spanning((30.0, 20.0), (40.0, 30.0)), 12, vec![0]),
            (spanning((30.0, 20.0), (150.0, 30.0)), 12, vec![0]),
            (spanning((30.0, 20.0), (154.0, 30.0)), 12, vec![0, 1]),
            (spanning((145.0, 20.0), (146.0, 30.0)), 12, vec![]),
            (spanning((30.0, 20.0), (300.0, 30.0)), 12, vec![0, 1, 2]),
            (spanning((30.0, 20.0), (60.0, 150.0)), 12, vec![0, 4]),
            (
                spanning((30.0, 20.0), (300.0, 150.0)),
                12,
                vec![0, 1, 2, 4, 5, 6],
            ),
            (
                spanning((300.0, 150.0), (30.0, 20.0)),
                12,
                vec![0, 1, 2, 4, 5, 6],
            ),
            (spanning((0.0, 0.0), (600.0, 400.0)), 12, (0..12).collect()),
            (spanning((0.0, 0.0), (600.0, 400.0)), 10, (0..10).collect()),
            (spanning((430.0, 216.0), (600.0, 400.0)), 11, vec![]),
            (spanning((430.0, 216.0), (600.0, 400.0)), 12, vec![11]),
            (spanning((30.0, 105.0), (60.0, 111.0)), 12, vec![]),
            (spanning((30.0, 104.0), (60.0, 112.0)), 12, vec![4]),
            (spanning((0.0, 0.0), (10.0, 400.0)), 12, vec![]),
            (spanning((600.0, 0.0), (700.0, 400.0)), 12, vec![]),
        ];
        for (boxed, total, cells) in cases {
            assert_eq!(GRID.touched(boxed, total), cells, "{boxed:?} {total}");
        }
    }

    #[test]
    fn touched_is_exactly_the_tiles_whose_rectangles_the_box_touches() {
        let boxes = [
            spanning((0.0, 0.0), (1.0, 1.0)),
            spanning((100.0, 90.0), (300.0, 400.0)),
            spanning((17.0, 111.0), (19.0, 113.0)),
            spanning((270.0, 100.0), (430.0, 230.0)),
            spanning((-30.0, 200.0), (600.0, 205.0)),
            spanning((140.0, 8.0), (154.0, 104.0)),
        ];
        for tiles in [LIST, GRID] {
            for total in [0, 1, 4, 5, 23] {
                for boxed in boxes {
                    let brute = (0..total)
                        .filter(|&index| boxed.touches(tiles.rect(index)))
                        .collect::<Vec<_>>();
                    assert_eq!(
                        tiles.touched(boxed, total),
                        brute,
                        "{tiles:?} {total} {boxed:?}"
                    );
                }
            }
        }
    }
}
