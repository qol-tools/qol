pub const ROW_H: f32 = 82.0;
pub const VISIBLE: usize = 3;
pub const PAD_TOP: f32 = 18.0;
pub const DOT_OFFSET: f32 = PAD_TOP + 11.5;
pub const TRAVEL_MS: u64 = 260;
pub const DRAIN_MS: u64 = 140;
pub const SETTLE_MS: u64 = TRAVEL_MS + DRAIN_MS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Travel,
    Drain,
}

pub fn ease_travel(delta: f32) -> f32 {
    if delta < 0.5 {
        2.0 * delta * delta
    } else {
        let x = -2.0 * delta + 2.0;
        1.0 - x * x / 2.0
    }
}

pub fn ease_drain(delta: f32) -> f32 {
    1.0 - (1.0 - delta).powi(5)
}

pub fn viewport_height(head_h: f32) -> f32 {
    head_h + (VISIBLE - 1) as f32 * ROW_H
}

pub fn row_top(index: f32, head_h: f32) -> f32 {
    if index <= 0.0 {
        0.0
    } else if index < 1.0 {
        index * head_h
    } else {
        head_h + (index - 1.0) * ROW_H
    }
}

pub fn dot_center(pos: f32, head_h: f32, head_dot: f32) -> f32 {
    if pos <= 1.0 {
        head_dot + (head_h + DOT_OFFSET - head_dot) * pos
    } else {
        head_h + DOT_OFFSET + (pos - 1.0) * ROW_H
    }
}

pub fn track_offset(pos: f32, len: usize, head_h: f32) -> f32 {
    -row_top(
        (pos - 1.0).clamp(0.0, len.saturating_sub(VISIBLE) as f32),
        head_h,
    )
}

pub fn segment(
    from: f32,
    to: usize,
    phase: Phase,
    delta: f32,
    head_h: f32,
    head_dot: f32,
) -> (f32, f32) {
    let a = dot_center(from, head_h, head_dot);
    let b = dot_center(to as f32, head_h, head_dot);
    match phase {
        Phase::Travel if b > a => (a, (b - a) * delta),
        Phase::Travel => (a - (a - b) * delta, (a - b) * delta),
        Phase::Drain if b > a => (a + (b - a) * delta, (b - a) * (1.0 - delta)),
        Phase::Drain => (b, (a - b) * (1.0 - delta)),
    }
}

pub fn head_center(
    from: f32,
    to: usize,
    phase: Phase,
    delta: f32,
    head_h: f32,
    head_dot: f32,
) -> Option<f32> {
    match phase {
        Phase::Travel => {
            let a = dot_center(from, head_h, head_dot);
            let b = dot_center(to as f32, head_h, head_dot);
            Some(a + (b - a) * delta)
        }
        Phase::Drain => None,
    }
}

pub fn lit(to: usize, phase: Phase) -> Option<usize> {
    match phase {
        Phase::Travel => None,
        Phase::Drain => Some(to),
    }
}

pub fn here(from_index: usize, to: usize, phase: Phase) -> usize {
    match phase {
        Phase::Travel => from_index,
        Phase::Drain => to,
    }
}

pub fn fill(phase: Phase, delta: f32) -> f32 {
    match phase {
        Phase::Travel => 0.0,
        Phase::Drain => delta,
    }
}

pub fn position_at(from: f32, to: usize, elapsed_ms: u64) -> f32 {
    if elapsed_ms < TRAVEL_MS {
        from + (to as f32 - from) * ease_travel(elapsed_ms as f32 / TRAVEL_MS as f32)
    } else {
        to as f32
    }
}

pub fn slide(from: f32, to: usize, len: usize, phase: Phase, delta: f32, head_h: f32) -> f32 {
    match phase {
        Phase::Travel => {
            let start = track_offset(from, len, head_h);
            start + (track_offset(to as f32, len, head_h) - start) * delta
        }
        Phase::Drain => track_offset(to as f32, len, head_h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: usize = 20;
    const TALL_HEAD_H: f32 = 116.0;
    const TALL_HEAD_DOT: f32 = 36.0;

    fn heads() -> [(f32, f32); 2] {
        [(ROW_H, DOT_OFFSET), (TALL_HEAD_H, TALL_HEAD_DOT)]
    }

    fn swept() -> impl Iterator<Item = f32> {
        (0..=STEPS).map(|step| step as f32 / STEPS as f32)
    }

    #[test]
    fn a_uniform_head_reproduces_the_flat_row_maths_exactly() {
        for pos in [-1.0f32, -0.5, 0.0, 0.25, 1.0, 1.5, 4.0, 9.0] {
            assert_eq!(
                dot_center(pos, ROW_H, DOT_OFFSET),
                pos * ROW_H + DOT_OFFSET,
                "pos={pos}"
            );
        }
        for len in [0usize, 1, 3, 10] {
            for pos in [0.0f32, 1.0, 2.0, 2.5, 3.0, 9.0] {
                assert_eq!(
                    track_offset(pos, len, ROW_H),
                    -(pos - 1.0).clamp(0.0, len.saturating_sub(VISIBLE) as f32) * ROW_H,
                    "len={len} pos={pos}"
                );
            }
        }
        for index in [0.0f32, 0.5, 1.0, 2.0, 4.5] {
            assert_eq!(row_top(index, ROW_H), index * ROW_H, "index={index}");
        }
        assert_eq!(viewport_height(ROW_H), VISIBLE as f32 * ROW_H);
    }

    #[test]
    fn a_taller_head_puts_the_second_dot_below_the_head_row() {
        assert_eq!(dot_center(0.0, TALL_HEAD_H, TALL_HEAD_DOT), TALL_HEAD_DOT);
        assert_eq!(
            dot_center(1.0, TALL_HEAD_H, TALL_HEAD_DOT),
            TALL_HEAD_H + DOT_OFFSET
        );
        assert_eq!(
            dot_center(2.0, TALL_HEAD_H, TALL_HEAD_DOT),
            TALL_HEAD_H + ROW_H + DOT_OFFSET
        );
        assert_eq!(row_top(0.0, TALL_HEAD_H), 0.0);
        assert_eq!(row_top(0.5, TALL_HEAD_H), 0.5 * TALL_HEAD_H);
        assert_eq!(row_top(1.0, TALL_HEAD_H), TALL_HEAD_H);
        assert_eq!(row_top(2.0, TALL_HEAD_H), TALL_HEAD_H + ROW_H);
        assert_eq!(
            viewport_height(TALL_HEAD_H),
            TALL_HEAD_H + (VISIBLE - 1) as f32 * ROW_H
        );
    }

    #[test]
    fn dot_center_interpolates_between_the_anchor_dots_and_extrapolates_off_the_nearest_segment() {
        let first = dot_center(0.0, TALL_HEAD_H, TALL_HEAD_DOT);
        let second = dot_center(1.0, TALL_HEAD_H, TALL_HEAD_DOT);
        assert_eq!(
            dot_center(0.5, TALL_HEAD_H, TALL_HEAD_DOT),
            (first + second) / 2.0
        );
        assert_eq!(
            dot_center(-1.0, TALL_HEAD_H, TALL_HEAD_DOT),
            first - (second - first)
        );
        assert_eq!(
            dot_center(2.5, TALL_HEAD_H, TALL_HEAD_DOT),
            second + ROW_H + ROW_H / 2.0
        );
    }

    #[test]
    fn segment_has_zero_height_where_the_head_leaves_and_where_it_arrives() {
        for (head_h, head_dot) in heads() {
            for (from, to) in [(0.0f32, 4usize), (4.0, 0), (2.0, 2)] {
                assert_eq!(
                    segment(from, to, Phase::Travel, 0.0, head_h, head_dot).1,
                    0.0
                );
                assert_eq!(
                    segment(from, to, Phase::Drain, 1.0, head_h, head_dot).1,
                    0.0
                );
            }
        }
    }

    #[test]
    fn segment_spans_the_full_distance_at_the_phase_boundary() {
        for (head_h, head_dot) in heads() {
            for (from, to) in [(0.0f32, 4usize), (4.0, 0)] {
                let a = dot_center(from, head_h, head_dot);
                let b = dot_center(to as f32, head_h, head_dot);
                let travel = segment(from, to, Phase::Travel, 1.0, head_h, head_dot);
                let drain = segment(from, to, Phase::Drain, 0.0, head_h, head_dot);
                for (top, height) in [travel, drain] {
                    assert_eq!(
                        (top, top + height),
                        (a.min(b), a.max(b)),
                        "from={from} to={to}"
                    );
                }
            }
        }
    }

    #[test]
    fn segment_stays_inside_the_closed_interval_between_the_dots() {
        for (head_h, head_dot) in heads() {
            for (from, to) in [(0.0f32, 4usize), (4.0, 0), (2.0, 2)] {
                let a = dot_center(from, head_h, head_dot);
                let b = dot_center(to as f32, head_h, head_dot);
                let (low, high) = (a.min(b), a.max(b));
                for phase in [Phase::Travel, Phase::Drain] {
                    for delta in swept() {
                        let (top, height) = segment(from, to, phase, delta, head_h, head_dot);
                        let far = top + height;
                        assert!(
                            top >= low && far <= high + 0.01,
                            "from={from} to={to} phase={phase:?} delta={delta}: {top}..{far}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn track_offset_holds_at_the_top_and_stops_at_the_last_window() {
        let len = 10;
        let end = -(len as f32 - VISIBLE as f32) * ROW_H;
        for (pos, expected) in [
            (0.0f32, 0.0f32),
            (1.0, 0.0),
            (2.0, -ROW_H),
            (3.0, -2.0 * ROW_H),
            (8.0, end),
            (9.0, end),
        ] {
            assert_eq!(track_offset(pos, len, ROW_H), expected, "pos={pos}");
        }
    }

    #[test]
    fn a_trail_no_taller_than_the_viewport_never_scrolls() {
        for head_h in [ROW_H, TALL_HEAD_H] {
            for len in 0..=VISIBLE {
                for selected in 0..=(len + 2) {
                    assert_eq!(
                        track_offset(selected as f32, len, head_h),
                        0.0,
                        "head_h={head_h} len={len} selected={selected}"
                    );
                }
            }
        }
    }

    #[test]
    fn lit_here_and_fill_flip_exactly_at_the_phase_boundary() {
        for (from, to) in [(0.0f32, 4usize), (4.0, 0), (2.0, 2)] {
            assert_eq!(lit(to, Phase::Travel), None, "from={from} to={to}");
            assert_eq!(lit(to, Phase::Drain), Some(to), "from={from} to={to}");
            assert_eq!(
                here(from as usize, to, Phase::Travel),
                from as usize,
                "from={from} to={to}"
            );
            assert_eq!(
                here(from as usize, to, Phase::Drain),
                to,
                "from={from} to={to}"
            );
            assert_eq!(fill(Phase::Travel, 0.5), 0.0);
            assert_eq!(fill(Phase::Drain, 0.5), 0.5);
        }
    }

    #[test]
    fn slide_is_continuous_across_the_boundary_and_holds_through_the_drain() {
        for (head_h, _) in heads() {
            for (from, to, len) in [
                (0.0f32, 4usize, 10usize),
                (4.0, 0, 10),
                (1.0, 1, 3),
                (0.0, 2, 3),
                (3.0, 3, 3),
            ] {
                let arrived = slide(from, to, len, Phase::Travel, 1.0, head_h);
                assert_eq!(
                    arrived,
                    slide(from, to, len, Phase::Drain, 0.0, head_h),
                    "head_h={head_h} from={from} to={to} len={len}"
                );
                assert_eq!(arrived, track_offset(to as f32, len, head_h));
                for delta in swept() {
                    assert_eq!(
                        slide(from, to, len, Phase::Drain, delta, head_h),
                        track_offset(to as f32, len, head_h),
                        "head_h={head_h} from={from} to={to} len={len} delta={delta}"
                    );
                }
                let halfway =
                    (track_offset(from, len, head_h) + track_offset(to as f32, len, head_h)) / 2.0;
                assert_eq!(slide(from, to, len, Phase::Travel, 0.5, head_h), halfway);
            }
        }
    }

    #[test]
    fn position_at_starts_at_from_ends_at_to_and_is_strictly_between_halfway() {
        assert_eq!(position_at(0.0, 4, 0), 0.0);
        assert_eq!(position_at(0.0, 4, TRAVEL_MS), 4.0);
        assert_eq!(position_at(0.0, 4, TRAVEL_MS + 1_000), 4.0);
        let mid = position_at(0.0, 4, TRAVEL_MS / 2);
        assert!(mid > 0.0 && mid < 4.0, "mid={mid}");
    }

    #[test]
    fn track_offset_at_a_fractional_position_lies_between_the_neighbouring_whole_positions() {
        for head_h in [ROW_H, TALL_HEAD_H] {
            let len = 10;
            let near = track_offset(2.0, len, head_h);
            let far = track_offset(3.0, len, head_h);
            let mid = track_offset(2.5, len, head_h);
            assert!(
                mid < near && mid > far,
                "head_h={head_h} near={near} mid={mid} far={far}"
            );
        }
    }

    #[test]
    fn segment_and_head_center_from_a_fractional_from_stay_inside_the_two_dot_centres() {
        for (head_h, head_dot) in heads() {
            for (from, to) in [(1.5f32, 4usize), (3.5, 0), (2.5, 2)] {
                let a = dot_center(from, head_h, head_dot);
                let b = dot_center(to as f32, head_h, head_dot);
                let (low, high) = (a.min(b), a.max(b));
                for phase in [Phase::Travel, Phase::Drain] {
                    for delta in swept() {
                        let (top, height) = segment(from, to, phase, delta, head_h, head_dot);
                        let far = top + height;
                        assert!(
                            top >= low && far <= high + 0.01,
                            "from={from} to={to} phase={phase:?} delta={delta}: {top}..{far}"
                        );
                        if let Some(center) = head_center(from, to, phase, delta, head_h, head_dot)
                        {
                            assert!(
                                center >= low && center <= high,
                                "from={from} to={to} phase={phase:?} delta={delta}: {center}"
                            );
                        }
                    }
                }
            }
        }
    }
}
