use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;

pub struct ShakeDetector {
    axes: [AxisSwing; 2],
    reversals: VecDeque<Instant>,
    last_at: Option<Instant>,
    thresholds: Thresholds,
    window: Duration,
    calm_duration: Duration,
    scale_factor: f32,
    grow_step: f32,
    shrink_step: f32,
    current_scale: f32,
    growing: bool,
    last_shake: Option<Instant>,
}

pub struct ScaleUpdate {
    pub scale_changed: Option<f32>,
    pub event: Option<ScaleEvent>,
}

#[derive(Clone, Copy)]
pub struct MotionSample {
    at: Instant,
    dx: i32,
    dy: i32,
}

#[derive(Clone, Copy)]
pub enum ScaleEvent {
    Grew { reversals: usize },
    Restored,
}

struct Thresholds {
    reversals: usize,
    regrow_reversals: usize,
    swing_min: u32,
    speed_min: f64,
}

#[derive(Clone, Copy)]
struct SwingRules {
    travel_min: u32,
    speed_min: f64,
}

#[derive(Default)]
struct AxisSwing {
    direction: i32,
    travel: u32,
    peak_speed: f64,
}

impl AxisSwing {
    fn advance(&mut self, delta: i32, speed: f64, rules: SwingRules) -> bool {
        if delta == 0 {
            return false;
        }
        let sign = delta.signum();
        if sign == self.direction {
            self.travel = self.travel.saturating_add(delta.unsigned_abs());
            self.peak_speed = self.peak_speed.max(speed);
            return false;
        }
        let completed = self.travel;
        let completed_speed = self.peak_speed;
        self.direction = sign;
        self.travel = delta.unsigned_abs();
        self.peak_speed = speed;
        completed >= rules.travel_min && completed_speed >= rules.speed_min
    }
}

impl ShakeDetector {
    pub fn new(config: &Config) -> Self {
        let scale_factor = config.scale_factor as f32;
        Self {
            axes: [AxisSwing::default(), AxisSwing::default()],
            reversals: VecDeque::new(),
            last_at: None,
            thresholds: Thresholds::from(config),
            window: Duration::from_millis(config.shake_window_ms),
            calm_duration: Duration::from_millis(config.calm_duration_ms),
            scale_factor,
            grow_step: (scale_factor - 1.0) / (config.restore_steps as f32 / 2.0).max(1.0),
            shrink_step: (scale_factor - 1.0) / (config.restore_steps as f32).max(1.0),
            current_scale: 1.0,
            growing: false,
            last_shake: None,
        }
    }

    pub fn record(&mut self, sample: MotionSample) -> ScaleUpdate {
        self.track_reversals(sample);
        self.last_at = Some(sample.at);
        self.trim(sample.at);
        self.update(sample.at)
    }

    fn track_reversals(&mut self, sample: MotionSample) {
        let elapsed = self
            .last_at
            .map(|last| (sample.at - last).as_secs_f64())
            .unwrap_or(f64::INFINITY);
        let rules = SwingRules {
            travel_min: self.thresholds.swing_min,
            speed_min: self.thresholds.speed_min,
        };
        for (axis, delta) in [(0, sample.dx), (1, sample.dy)] {
            let speed = axis_speed(delta, elapsed);
            if self.axes[axis].advance(delta, speed, rules) {
                self.reversals.push_back(sample.at);
            }
        }
    }

    fn trim(&mut self, now: Instant) {
        while self
            .reversals
            .front()
            .is_some_and(|at| now - *at > self.window)
        {
            self.reversals.pop_front();
        }
    }

    fn update(&mut self, now: Instant) -> ScaleUpdate {
        if self.is_shake() {
            self.growing = true;
            self.last_shake = Some(now);
        } else {
            self.maybe_stop_growing(now);
        }

        let previous_scale = self.current_scale;
        let target_scale = if self.growing { self.scale_factor } else { 1.0 };
        let next_scale = self.next_scale(target_scale);
        self.current_scale = next_scale;

        ScaleUpdate {
            scale_changed: scale_changed(previous_scale, next_scale),
            event: scale_event(previous_scale, next_scale, self.reversals.len()),
        }
    }

    fn is_shake(&self) -> bool {
        if !self.growing && self.is_scaled() {
            return self.reversals.len() >= self.thresholds.regrow_reversals;
        }
        self.reversals.len() >= self.thresholds.reversals
    }

    fn maybe_stop_growing(&mut self, now: Instant) {
        if !self.growing || self.current_scale < self.scale_factor - f32::EPSILON {
            return;
        }
        if self
            .last_shake
            .is_some_and(|last_shake| now - last_shake > self.calm_duration)
        {
            self.growing = false;
        }
    }

    fn is_scaled(&self) -> bool {
        self.current_scale > 1.0 + f32::EPSILON
    }

    fn next_scale(&self, target: f32) -> f32 {
        if target > self.current_scale {
            return (self.current_scale + self.grow_step).min(target);
        }
        if target < self.current_scale {
            return (self.current_scale - self.shrink_step).max(target);
        }
        self.current_scale
    }
}

impl MotionSample {
    pub fn new(at: Instant, dx: i32, dy: i32) -> Self {
        Self { at, dx, dy }
    }
}

impl From<&Config> for Thresholds {
    fn from(config: &Config) -> Self {
        Self {
            reversals: config.shake_reversals as usize,
            regrow_reversals: config.regrow_reversals as usize,
            swing_min: config.swing_min_px,
            speed_min: config.swing_min_speed,
        }
    }
}

fn axis_speed(delta: i32, elapsed_secs: f64) -> f64 {
    if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
        return 0.0;
    }
    f64::from(delta.unsigned_abs()) / elapsed_secs
}

fn scale_changed(previous: f32, current: f32) -> Option<f32> {
    if (current - previous).abs() > f32::EPSILON {
        return Some(current);
    }
    None
}

fn scale_event(previous: f32, current: f32, reversals: usize) -> Option<ScaleEvent> {
    let was_scaled = previous > 1.0 + f32::EPSILON;
    let is_scaled = current > 1.0 + f32::EPSILON;
    match (was_scaled, is_scaled) {
        (false, true) => Some(ScaleEvent::Grew { reversals }),
        (true, false) => Some(ScaleEvent::Restored),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type ShakeCase = (&'static str, Vec<(u64, i32, i32)>, bool);

    fn config() -> Config {
        Config {
            shake_reversals: 5,
            regrow_reversals: 3,
            shake_window_ms: 900,
            swing_min_px: 40,
            swing_min_speed: 1200.0,
            scale_factor: 4,
            calm_duration_ms: 650,
            restore_steps: 18,
        }
    }

    fn feed(detector: &mut ShakeDetector, t0: Instant, trace: &[(u64, i32, i32)]) -> Option<u64> {
        for (ms, dx, dy) in trace {
            let at = t0 + Duration::from_millis(*ms);
            let update = detector.record(MotionSample::new(at, *dx, *dy));
            if let Some(ScaleEvent::Grew { .. }) = update.event {
                return Some(*ms);
            }
        }
        None
    }

    fn wiggle(amplitude: i32, half_period_ms: u64, duration_ms: u64) -> Vec<(u64, i32, i32)> {
        let mut trace = Vec::new();
        let mut t = 0;
        let mut sign = 1;
        while t < duration_ms {
            let steps = (half_period_ms / 16).max(1);
            let step_px = amplitude / steps as i32;
            for _ in 0..steps {
                t += 16;
                trace.push((t, step_px * sign, 0));
            }
            sign = -sign;
        }
        trace
    }

    #[test]
    fn shake_trigger_table() {
        let cases: [ShakeCase; 8] = [
            ("brisk wiggle triggers", wiggle(120, 96, 2000), true),
            ("vigorous wide shake triggers", wiggle(240, 64, 2000), true),
            ("big fast arm shake triggers", wiggle(700, 125, 2000), true),
            ("slow fiddling never triggers", wiggle(60, 96, 3000), false),
            (
                "straight glide never triggers",
                (1..60).map(|i| (i * 16, 40, 0)).collect(),
                false,
            ),
            (
                "wide 2Hz scrubbing never triggers",
                wiggle(800, 250, 3000),
                false,
            ),
            (
                "sub-threshold tremor never triggers",
                wiggle(6, 96, 3000),
                false,
            ),
            (
                "vertical wiggle triggers too",
                wiggle(120, 96, 2000)
                    .iter()
                    .map(|(t, dx, dy)| (*t, *dy, *dx))
                    .collect(),
                true,
            ),
        ];
        for (label, trace, expect_grow) in cases {
            let mut detector = ShakeDetector::new(&config());
            let grew_at = feed(&mut detector, Instant::now(), &trace);
            assert_eq!(
                grew_at.is_some(),
                expect_grow,
                "case: {label} grew_at={grew_at:?}"
            );
        }
    }

    #[test]
    fn moderate_wiggle_triggers_within_a_second() {
        let mut detector = ShakeDetector::new(&config());
        let grew_at = feed(&mut detector, Instant::now(), &wiggle(120, 96, 2000));
        let at = grew_at.expect("brisk wiggle must trigger");
        assert!(at <= 1000, "trigger should land within 1s, got {at}ms");
    }

    #[test]
    fn slow_reversals_outside_window_never_trigger() {
        // Direction changes 700ms apart: five reversals span far more than the
        // 900ms window, so the count never accumulates.
        let mut detector = ShakeDetector::new(&config());
        let grew_at = feed(&mut detector, Instant::now(), &wiggle(60, 700, 6000));
        assert_eq!(grew_at, None, "leisurely back-and-forth must not trigger");
    }

    #[test]
    fn cursor_shrinks_after_calm_and_regrows_on_three_reversals() {
        let mut detector = ShakeDetector::new(&config());
        let t0 = Instant::now();
        feed(&mut detector, t0, &wiggle(120, 96, 1200)).expect("initial grow");
        let mut t = Duration::from_millis(1200);
        let mut restored = false;
        for _ in 0..200 {
            t += Duration::from_millis(16);
            let update = detector.record(MotionSample::new(t0 + t, 0, 0));
            if let Some(ScaleEvent::Restored) = update.event {
                restored = true;
                break;
            }
            if let Some(ScaleEvent::Grew { .. }) = update.event {
                panic!("idle ticks must not regrow");
            }
            if update.scale_changed.is_some_and(|scale| scale < 4.0) && !detector.growing {
                // shrinking has begun: three quick reversals must regrow
                for dx in [40, -40, 40, -40, 40, -40] {
                    t += Duration::from_millis(16);
                    let update = detector.record(MotionSample::new(t0 + t, dx, 0));
                    if let Some(ScaleEvent::Grew { .. }) = update.event {
                        return;
                    }
                }
                assert!(
                    detector.growing,
                    "three reversals while shrinking must regrow"
                );
                return;
            }
        }
        assert!(restored, "cursor must eventually restore after calm");
    }
}
