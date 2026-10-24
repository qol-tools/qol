use std::time::{Duration, Instant};

pub const HOLD: Duration = Duration::from_millis(500);
pub const CHARGE_FROM: Duration = Duration::from_millis(100);
pub const SLOP: f32 = 4.0;

const SMALLEST: f32 = 0.2;
const LABEL_FROM: f32 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Boxing,
    Lifted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Stay,
    Box,
}

#[derive(Clone, Copy, Debug)]
pub struct Hold {
    origin: (f32, f32),
    at: Instant,
    on_row: bool,
    phase: Phase,
    announced: bool,
}

impl Hold {
    pub fn new(origin: (f32, f32), at: Instant, on_row: bool) -> Self {
        Self {
            origin,
            at,
            on_row,
            phase: Phase::Pending,
            announced: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn settle(&mut self, now: Instant) -> bool {
        self.advance(now);
        let lifted = self.phase == Phase::Lifted && !self.announced;
        self.announced |= lifted;
        lifted
    }

    fn advance(&mut self, now: Instant) {
        let held = now.saturating_duration_since(self.at) >= HOLD;
        if self.phase == Phase::Pending && self.on_row && held {
            self.phase = Phase::Lifted;
        }
    }

    pub fn moved(&mut self, to: (f32, f32), now: Instant) -> Move {
        self.advance(now);
        let travelled = (to.0 - self.origin.0).hypot(to.1 - self.origin.1);
        if self.phase == Phase::Pending && travelled > SLOP {
            self.phase = Phase::Boxing;
            return Move::Box;
        }
        Move::Stay
    }

    pub fn charge(&self, now: Instant) -> Option<f32> {
        if self.phase != Phase::Pending || !self.on_row {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.at);
        if elapsed < CHARGE_FROM || elapsed >= HOLD {
            return None;
        }
        let span = HOLD - CHARGE_FROM;
        Some((elapsed - CHARGE_FROM).as_secs_f32() / span.as_secs_f32())
    }
}

pub fn diameter(count: usize) -> f32 {
    match count {
        0..=9 => 22.0,
        10..=99 => 26.0,
        _ => 32.0,
    }
}

pub fn charge_look(eased: f32) -> (f32, f32) {
    (SMALLEST + (1.0 - SMALLEST) * eased, eased)
}

pub fn label_opacity(progress: f32) -> f32 {
    ((progress - LABEL_FROM) / (1.0 - LABEL_FROM)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{charge_look, diameter, label_opacity, Hold, Move, Phase};

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    fn ms(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn a_press_boxes_when_it_moves_past_four_pixels_before_the_hold_and_lifts_when_it_waits() {
        let cases = [
            (true, 0, (0.0, 0.0), Move::Stay, Phase::Pending),
            (true, 0, (4.0, 0.0), Move::Stay, Phase::Pending),
            (true, 0, (-4.0, 0.0), Move::Stay, Phase::Pending),
            (true, 0, (0.0, 4.0), Move::Stay, Phase::Pending),
            (true, 0, (2.8, 2.8), Move::Stay, Phase::Pending),
            (true, 0, (4.01, 0.0), Move::Box, Phase::Boxing),
            (true, 0, (0.0, -4.01), Move::Box, Phase::Boxing),
            (true, 0, (3.0, 3.0), Move::Box, Phase::Boxing),
            (true, 1, (50.0, 80.0), Move::Box, Phase::Boxing),
            (true, 250, (5.0, 0.0), Move::Box, Phase::Boxing),
            (true, 499, (5.0, 0.0), Move::Box, Phase::Boxing),
            (true, 499, (3.0, 0.0), Move::Stay, Phase::Pending),
            (true, 500, (0.0, 0.0), Move::Stay, Phase::Lifted),
            (true, 500, (3.0, 0.0), Move::Stay, Phase::Lifted),
            (true, 500, (30.0, 0.0), Move::Stay, Phase::Lifted),
            (true, 501, (30.0, 40.0), Move::Stay, Phase::Lifted),
            (true, 5000, (300.0, 0.0), Move::Stay, Phase::Lifted),
            (false, 0, (0.0, 0.0), Move::Stay, Phase::Pending),
            (false, 0, (4.0, 0.0), Move::Stay, Phase::Pending),
            (false, 0, (4.5, 0.0), Move::Box, Phase::Boxing),
            (false, 499, (9.0, 0.0), Move::Box, Phase::Boxing),
            (false, 500, (9.0, 0.0), Move::Box, Phase::Boxing),
            (false, 60_000, (9.0, 0.0), Move::Box, Phase::Boxing),
            (false, 60_000, (1.0, 1.0), Move::Stay, Phase::Pending),
        ];
        for (on_row, millis, (dx, dy), want_move, want_phase) in cases {
            let start = Instant::now();
            let mut hold = Hold::new((100.0, 200.0), start, on_row);
            let to = (100.0 + dx, 200.0 + dy);
            assert_eq!(
                hold.moved(to, ms(start, millis)),
                want_move,
                "row {on_row} {millis} ms {dx},{dy}"
            );
            assert_eq!(
                hold.phase(),
                want_phase,
                "row {on_row} {millis} ms {dx},{dy}"
            );
        }
    }

    #[test]
    fn settling_lifts_a_row_press_at_exactly_the_hold_and_never_an_empty_one() {
        let cases = [
            (true, 0, false),
            (true, 99, false),
            (true, 499, false),
            (true, 500, true),
            (true, 501, true),
            (true, 9000, true),
            (false, 0, false),
            (false, 500, false),
            (false, 9000, false),
        ];
        for (on_row, millis, lifted) in cases {
            let start = Instant::now();
            let mut hold = Hold::new((0.0, 0.0), start, on_row);
            assert_eq!(hold.settle(ms(start, millis)), lifted, "{on_row} {millis}");
            assert_eq!(hold.phase() == Phase::Lifted, lifted, "{on_row} {millis}");
        }
    }

    #[test]
    fn a_lift_is_reported_once_even_when_a_move_saw_it_first() {
        let start = Instant::now();
        let mut settled = Hold::new((0.0, 0.0), start, true);
        let reports: Vec<bool> = [499, 500, 501, 900]
            .into_iter()
            .map(|millis| settled.settle(ms(start, millis)))
            .collect();
        assert_eq!(reports, [false, true, false, false]);

        let mut moved = Hold::new((0.0, 0.0), start, true);
        assert_eq!(moved.moved((1.0, 0.0), ms(start, 600)), Move::Stay);
        assert_eq!(moved.phase(), Phase::Lifted);
        assert!(moved.settle(ms(start, 601)));
        assert!(!moved.settle(ms(start, 602)));

        let mut empty = Hold::new((0.0, 0.0), start, false);
        assert!(!empty.settle(ms(start, 600)));
        assert!(!empty.settle(ms(start, 601)));
    }

    #[test]
    fn a_time_before_the_press_does_not_lift_or_panic() {
        let start = Instant::now() + Duration::from_secs(10);
        let mut hold = Hold::new((0.0, 0.0), start, true);
        assert!(!hold.settle(start - Duration::from_secs(1)));
        assert_eq!(hold.charge(start - Duration::from_secs(1)), None);
    }

    #[test]
    fn a_box_never_lifts_and_a_lift_never_boxes() {
        let start = Instant::now();
        let mut boxing = Hold::new((0.0, 0.0), start, true);
        assert_eq!(boxing.moved((20.0, 0.0), ms(start, 10)), Move::Box);
        assert!(!boxing.settle(ms(start, 9000)));
        assert_eq!(boxing.moved((90.0, 0.0), ms(start, 9001)), Move::Stay);
        assert_eq!(boxing.phase(), Phase::Boxing);

        let mut lifted = Hold::new((0.0, 0.0), start, true);
        assert!(lifted.settle(ms(start, 600)));
        assert_eq!(lifted.moved((90.0, 0.0), ms(start, 601)), Move::Stay);
        assert_eq!(lifted.phase(), Phase::Lifted);
    }

    #[test]
    fn a_slow_drift_adds_up_from_where_the_press_began() {
        let start = Instant::now();
        let mut hold = Hold::new((0.0, 0.0), start, true);
        let path = [
            (1.0, 0.0, Move::Stay),
            (2.0, 0.0, Move::Stay),
            (3.0, 0.0, Move::Stay),
            (4.0, 0.0, Move::Stay),
            (5.0, 0.0, Move::Box),
        ];
        for (index, (x, y, want)) in path.into_iter().enumerate() {
            let at = ms(start, 50 * (index as u64 + 1));
            assert_eq!(hold.moved((x, y), at), want, "{x}");
        }
    }

    #[test]
    fn the_charge_runs_from_one_hundred_to_five_hundred_milliseconds_of_a_still_row_press() {
        let cases = [
            (0, None),
            (50, None),
            (99, None),
            (100, Some(0.0)),
            (200, Some(0.25)),
            (300, Some(0.5)),
            (400, Some(0.75)),
            (499, Some(0.9975)),
            (500, None),
            (800, None),
        ];
        for (millis, want) in cases {
            let start = Instant::now();
            let hold = Hold::new((0.0, 0.0), start, true);
            let got = hold.charge(ms(start, millis));
            match (got, want) {
                (Some(got), Some(want)) => assert!(close(got, want), "{millis}: {got}"),
                (got, want) => assert_eq!(got, want, "{millis}"),
            }
        }
    }

    #[test]
    fn nothing_charges_for_an_empty_press_a_box_or_a_lift() {
        let start = Instant::now();
        let at = ms(start, 300);
        assert_eq!(Hold::new((0.0, 0.0), start, false).charge(at), None);
        let mut boxing = Hold::new((0.0, 0.0), start, true);
        boxing.moved((40.0, 0.0), ms(start, 200));
        assert_eq!(boxing.charge(at), None);
        let mut lifted = Hold::new((0.0, 0.0), start, true);
        lifted.settle(ms(start, 500));
        assert_eq!(lifted.charge(ms(start, 520)), None);
    }

    #[test]
    fn the_circle_stays_round_by_growing_with_the_digits_of_the_count() {
        let cases = [
            (0, 22.0),
            (1, 22.0),
            (2, 22.0),
            (3, 22.0),
            (9, 22.0),
            (10, 26.0),
            (42, 26.0),
            (99, 26.0),
            (100, 32.0),
            (999, 32.0),
            (1000, 32.0),
            (123_456, 32.0),
        ];
        for (count, want) in cases {
            assert_eq!(diameter(count), want, "{count}");
        }
    }

    #[test]
    fn the_circle_grows_from_a_fifth_of_its_size_and_fades_in_with_the_ease() {
        let cases = [
            (0.0, 0.2, 0.0),
            (0.25, 0.4, 0.25),
            (0.5, 0.6, 0.5),
            (1.0, 1.0, 1.0),
        ];
        for (eased, scale, opacity) in cases {
            let (got_scale, got_opacity) = charge_look(eased);
            assert!(close(got_scale, scale), "{eased}: {got_scale}");
            assert!(close(got_opacity, opacity), "{eased}: {got_opacity}");
        }
    }

    #[test]
    fn the_count_fades_in_over_the_last_forty_percent_of_the_charge() {
        let cases = [
            (0.0, 0.0),
            (0.3, 0.0),
            (0.6, 0.0),
            (0.7, 0.25),
            (0.8, 0.5),
            (0.9, 0.75),
            (1.0, 1.0),
            (1.5, 1.0),
            (-1.0, 0.0),
        ];
        for (progress, want) in cases {
            let got = label_opacity(progress);
            assert!(close(got, want), "{progress}: {got}");
        }
    }
}
