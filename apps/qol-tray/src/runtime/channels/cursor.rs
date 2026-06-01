use std::time::Duration;

use super::super::Channel;
use crate::desktop_state::SharedPlatform;

const MIN_INTERVAL: Duration = Duration::from_millis(16);
const NOISE_PX: f32 = 1.0;
const INTENTIONAL_PX: f32 = 20.0;
const REST_IDLE_POLLS: u32 = 5;

pub(crate) struct CursorChannel {
    platform: SharedPlatform,
    last_pos: Option<(f32, f32)>,
    current_pos: Option<(f32, f32)>,
    accumulated_px: f32,
    idle_polls: u32,
}

impl CursorChannel {
    pub(crate) fn new(platform: SharedPlatform) -> Self {
        Self {
            platform,
            last_pos: None,
            current_pos: None,
            accumulated_px: 0.0,
            idle_polls: 0,
        }
    }

    pub(crate) fn position(&self) -> Option<(f32, f32)> {
        self.current_pos
    }
}

impl Channel for CursorChannel {
    fn poll(&mut self) -> bool {
        let pos = self.platform.cursor_position();
        self.current_pos = pos;

        let intentional = match (pos, self.last_pos) {
            (Some((x, y)), Some((lx, ly))) => {
                let delta = (x - lx).abs() + (y - ly).abs();
                if delta > NOISE_PX {
                    self.accumulated_px += delta;
                    self.idle_polls = 0;
                } else {
                    self.idle_polls = self.idle_polls.saturating_add(1);
                    if self.idle_polls >= REST_IDLE_POLLS {
                        self.accumulated_px = 0.0;
                    }
                }
                self.accumulated_px > INTENTIONAL_PX
            }
            (Some(_), None) => {
                self.accumulated_px = 0.0;
                self.idle_polls = 0;
                true
            }
            _ => {
                self.accumulated_px = 0.0;
                self.idle_polls = 0;
                false
            }
        };

        self.last_pos = pos;
        intentional
    }

    fn min_interval(&self) -> Duration {
        MIN_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use proptest::prelude::*;
    use qol_runtime::MonitorBounds;

    use super::super::super::Channel;
    use super::*;
    use crate::desktop_state::Platform;

    struct ScriptedPlatform {
        positions: Mutex<Vec<Option<(f32, f32)>>>,
    }

    impl ScriptedPlatform {
        fn new(positions: Vec<Option<(f32, f32)>>) -> Arc<Self> {
            Arc::new(Self {
                positions: Mutex::new(positions),
            })
        }
    }

    impl Platform for ScriptedPlatform {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            let mut q = self.positions.lock().unwrap();
            if q.is_empty() {
                return None;
            }
            q.remove(0)
        }
        fn focused_window_bounds(&self) -> Option<MonitorBounds> {
            None
        }
        fn physical_monitors(&self) -> Vec<MonitorBounds> {
            Vec::new()
        }
    }

    fn run(positions: Vec<Option<(f32, f32)>>) -> (Vec<bool>, Vec<Option<(f32, f32)>>) {
        let platform = ScriptedPlatform::new(positions.clone());
        let mut ch = CursorChannel::new(platform);
        let mut changed = Vec::with_capacity(positions.len());
        let mut reported = Vec::with_capacity(positions.len());
        for _ in 0..positions.len() {
            changed.push(ch.poll());
            reported.push(ch.position());
        }
        (changed, reported)
    }

    #[test]
    fn min_interval_is_16ms() {
        let ch = CursorChannel::new(ScriptedPlatform::new(Vec::new()));
        assert_eq!(ch.min_interval(), Duration::from_millis(16));
    }

    #[test]
    fn new_starts_with_no_position() {
        let ch = CursorChannel::new(ScriptedPlatform::new(Vec::new()));
        assert_eq!(ch.position(), None);
    }

    type PollCase = (&'static str, Vec<Option<(f32, f32)>>, Vec<bool>);

    #[test]
    fn poll_transitions_table() {
        let cases: &[PollCase] = &[
            ("none stays none", vec![None, None, None], vec![false; 3]),
            (
                "first some is changed",
                vec![None, Some((10.0, 20.0))],
                vec![false, true],
            ),
            (
                "some to same is unchanged",
                vec![Some((5.0, 5.0)), Some((5.0, 5.0))],
                vec![true, false],
            ),
            (
                "some to none is unchanged",
                vec![Some((1.0, 1.0)), None],
                vec![true, false],
            ),
            (
                "none after some leaves last_pos none, then re-some triggers",
                vec![Some((1.0, 1.0)), None, Some((1.0, 1.0))],
                vec![true, false, true],
            ),
            (
                "tiny x delta below threshold",
                vec![Some((10.0, 10.0)), Some((10.5, 10.0))],
                vec![true, false],
            ),
            (
                "tiny y delta below threshold",
                vec![Some((10.0, 10.0)), Some((10.0, 10.5))],
                vec![true, false],
            ),
            (
                "delta exactly 1.0 not changed (strict greater-than)",
                vec![Some((10.0, 10.0)), Some((11.0, 10.0))],
                vec![true, false],
            ),
            (
                "delta just over noise threshold not yet intentional",
                vec![Some((10.0, 10.0)), Some((11.001, 10.0))],
                vec![true, false],
            ),
            (
                "single jump under intentional threshold",
                vec![Some((-5.0, -5.0)), Some((-7.5, -5.0))],
                vec![true, false],
            ),
            (
                "single jump over intentional threshold is intentional",
                vec![Some((0.0, 0.0)), Some((100.0, 0.0))],
                vec![true, true],
            ),
            (
                "cumulative accumulation crosses threshold",
                vec![
                    Some((0.0, 0.0)),
                    Some((5.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((15.0, 0.0)),
                    Some((22.0, 0.0)),
                ],
                vec![true, false, false, false, true],
            ),
            (
                "idle polls reset accumulator after rest window",
                vec![
                    Some((0.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((10.0, 0.0)),
                    Some((15.0, 0.0)),
                ],
                vec![true, false, false, false, false, false, false, false],
            ),
        ];

        for (label, positions, expected) in cases {
            let (got, _) = run(positions.clone());
            assert_eq!(&got, expected, "case: {label}");
        }
    }

    #[test]
    fn position_mirrors_last_polled_value_even_when_unchanged() {
        let positions = vec![
            Some((10.0, 10.0)),
            Some((10.5, 10.0)),
            None,
            Some((10.5, 10.0)),
        ];
        let (_, reported) = run(positions.clone());
        assert_eq!(reported, positions);
    }

    fn pos_strategy() -> impl Strategy<Value = Option<(f32, f32)>> {
        prop_oneof![
            Just(None),
            (-4000.0f32..4000.0, -4000.0f32..4000.0).prop_map(|(x, y)| Some((x, y))),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_position_equals_last_seen_pos(seq in proptest::collection::vec(pos_strategy(), 1..32)) {
            let (_, reported) = run(seq.clone());
            prop_assert_eq!(reported, seq);
        }

        #[test]
        fn prop_first_some_after_only_nones_returns_changed(
            n_leading in 0usize..6,
            x in -1000.0f32..1000.0,
            y in -1000.0f32..1000.0,
        ) {
            let mut seq: Vec<Option<(f32, f32)>> = vec![None; n_leading];
            seq.push(Some((x, y)));
            let (changed, _) = run(seq);
            for (i, c) in changed.iter().take(n_leading).enumerate() {
                prop_assert!(!c, "leading None #{i} must be unchanged");
            }
            prop_assert!(changed[n_leading], "first Some after Nones must be changed");
        }

        #[test]
        fn prop_small_single_jump_never_intentional(
            x in -1000.0f32..1000.0,
            y in -1000.0f32..1000.0,
            dx in -9.0f32..9.0,
            dy in -9.0f32..9.0,
        ) {
            // |dx|+|dy| <= 18 < 20 (INTENTIONAL_PX), so one jump can never be intentional.
            let seq = vec![Some((x, y)), Some((x + dx, y + dy))];
            let (changed, _) = run(seq);
            prop_assert_eq!(changed[0], true);
            prop_assert_eq!(changed[1], false);
        }

        #[test]
        fn prop_single_large_jump_is_intentional(
            x in -1000.0f32..1000.0,
            y in -1000.0f32..1000.0,
            dx in 30.0f32..200.0,
        ) {
            let seq = vec![Some((x, y)), Some((x + dx, y))];
            let (changed, _) = run(seq);
            prop_assert_eq!(changed, vec![true, true]);
        }

        #[test]
        fn prop_some_to_none_returns_false_then_some_returns_true(
            x in -1000.0f32..1000.0,
            y in -1000.0f32..1000.0,
            x2 in -1000.0f32..1000.0,
            y2 in -1000.0f32..1000.0,
        ) {
            let seq = vec![Some((x, y)), None, Some((x2, y2))];
            let (changed, _) = run(seq);
            prop_assert_eq!(changed, vec![true, false, true]);
        }

        #[test]
        fn prop_repeating_same_pos_only_first_is_changed(
            x in -1000.0f32..1000.0,
            y in -1000.0f32..1000.0,
            n in 2usize..10,
        ) {
            let seq = vec![Some((x, y)); n];
            let (changed, _) = run(seq);
            prop_assert!(changed[0]);
            for (i, c) in changed.iter().enumerate().skip(1) {
                prop_assert!(!c, "repeat #{i} must be unchanged");
            }
        }

        #[test]
        fn prop_min_interval_constant_regardless_of_state(seq in proptest::collection::vec(pos_strategy(), 0..16)) {
            let platform = ScriptedPlatform::new(seq.clone());
            let mut ch = CursorChannel::new(platform);
            for _ in 0..seq.len() {
                ch.poll();
                prop_assert_eq!(ch.min_interval(), Duration::from_millis(16));
            }
        }
    }
}
