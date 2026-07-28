use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;

const REVERSALS_TO_RESUME: u32 = 2;

pub struct ShakeDetector {
    trail: Trail,
    window: Duration,
    strictness: f64,
    regrow_strictness: f64,
    min_extent: f64,
    regrow_min_extent: f64,
    calm_duration: Duration,
    scale_factor: f32,
    grow_duration: Duration,
    shrink_duration: Duration,
    animation: Option<ScaleAnimation>,
    current_scale: f32,
    growing: bool,
    last_shake: Option<Instant>,
    shrink_reversals: u32,
}

#[derive(Clone, Copy)]
struct ScaleAnimation {
    from: f32,
    started: Instant,
    target: f32,
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
    Grew { tortuosity: f64 },
    Restored,
}

struct Trail {
    vertices: VecDeque<Vertex>,
    position: (f64, f64),
}

#[derive(Clone, Copy)]
struct Vertex {
    at: Instant,
    x: f64,
    y: f64,
}

struct TrailShape {
    extent: f64,
    tortuosity: f64,
}

struct TrailAdvance {
    shape: TrailShape,
    reversed: bool,
}

impl Trail {
    fn new() -> Self {
        Self {
            vertices: VecDeque::new(),
            position: (0.0, 0.0),
        }
    }

    fn reset(&mut self) {
        self.vertices.clear();
    }

    fn advance(&mut self, sample: MotionSample, window: Duration) -> Option<TrailAdvance> {
        self.position.0 += f64::from(sample.dx);
        self.position.1 += f64::from(sample.dy);
        while self
            .vertices
            .front()
            .is_some_and(|vertex| sample.at - vertex.at > window)
        {
            self.vertices.pop_front();
        }
        let vertex = Vertex {
            at: sample.at,
            x: self.position.0,
            y: self.position.1,
        };
        if self.extends_last_segment(vertex) {
            if let Some(last) = self.vertices.back_mut() {
                *last = vertex;
            }
            return self.advanced(false);
        }
        self.vertices.push_back(vertex);
        self.advanced(true)
    }

    fn advanced(&self, reversed: bool) -> Option<TrailAdvance> {
        Some(TrailAdvance {
            shape: self.shape()?,
            reversed,
        })
    }

    fn extends_last_segment(&self, next: Vertex) -> bool {
        let count = self.vertices.len();
        if count < 2 {
            return false;
        }
        let previous = self.vertices[count - 2];
        let last = self.vertices[count - 1];
        same_direction(last.x - previous.x, next.x - last.x)
            && same_direction(last.y - previous.y, next.y - last.y)
    }

    fn shape(&self) -> Option<TrailShape> {
        let first = self.vertices.front()?;
        let (mut min_x, mut max_x) = (first.x, first.x);
        let (mut min_y, mut max_y) = (first.y, first.y);
        let mut path = 0.0;
        let mut previous = *first;
        for vertex in self.vertices.iter().skip(1) {
            path += (vertex.x - previous.x).hypot(vertex.y - previous.y);
            min_x = min_x.min(vertex.x);
            max_x = max_x.max(vertex.x);
            min_y = min_y.min(vertex.y);
            max_y = max_y.max(vertex.y);
            previous = *vertex;
        }
        let extent = (max_x - min_x).hypot(max_y - min_y);
        if extent <= 0.0 {
            return None;
        }
        Some(TrailShape {
            extent,
            tortuosity: path / extent,
        })
    }
}

fn same_direction(previous: f64, current: f64) -> bool {
    (previous >= -1.0 && current >= -1.0) || (previous <= 1.0 && current <= 1.0)
}

impl ShakeDetector {
    pub fn new(config: &Config) -> Self {
        let scale_factor = config.scale_factor as f32;
        Self {
            trail: Trail::new(),
            window: Duration::from_millis(config.shake_window_ms),
            strictness: config.shake_strictness,
            regrow_strictness: config.regrow_strictness,
            min_extent: f64::from(config.shake_min_extent_px),
            regrow_min_extent: f64::from(config.regrow_min_extent_px),
            calm_duration: Duration::from_millis(config.calm_duration_ms),
            scale_factor,
            grow_duration: Duration::from_millis(u64::from(config.grow_ms.max(1))),
            shrink_duration: Duration::from_millis(u64::from(config.shrink_ms.max(1))),
            animation: None,
            current_scale: 1.0,
            growing: false,
            last_shake: None,
            shrink_reversals: 0,
        }
    }

    pub fn record(&mut self, sample: MotionSample) -> ScaleUpdate {
        let shake = self.detect(sample);
        self.update(sample.at, shake)
    }

    fn detect(&mut self, sample: MotionSample) -> Option<f64> {
        if sample.dx == 0 && sample.dy == 0 {
            return None;
        }
        let (strictness, min_extent) = if self.is_scaled() {
            (self.regrow_strictness, self.regrow_min_extent)
        } else {
            (self.strictness, self.min_extent)
        };
        let advance = self.trail.advance(sample, self.window)?;
        if self.is_shrinking() && !self.resumed_shaking(advance.reversed) {
            return None;
        }
        if advance.shape.extent < min_extent || advance.shape.tortuosity <= strictness {
            return None;
        }
        Some(advance.shape.tortuosity)
    }

    fn is_scaled(&self) -> bool {
        self.current_scale > 1.0 + f32::EPSILON
    }

    fn is_shrinking(&self) -> bool {
        self.is_scaled() && !self.growing
    }

    fn resumed_shaking(&mut self, reversed: bool) -> bool {
        if !reversed {
            return false;
        }
        self.shrink_reversals += 1;
        self.shrink_reversals >= REVERSALS_TO_RESUME
    }

    fn update(&mut self, now: Instant, shake: Option<f64>) -> ScaleUpdate {
        if shake.is_some() {
            self.growing = true;
            self.last_shake = Some(now);
            self.shrink_reversals = 0;
        } else {
            self.maybe_stop_growing(now);
        }

        let previous_scale = self.current_scale;
        let target_scale = if self.growing { self.scale_factor } else { 1.0 };
        let next_scale = self.next_scale(target_scale, now);
        self.current_scale = next_scale;

        let event = scale_event(previous_scale, next_scale, shake);
        if matches!(event, Some(ScaleEvent::Restored)) {
            self.trail.reset();
        }

        ScaleUpdate {
            scale_changed: scale_changed(previous_scale, next_scale),
            event,
        }
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

    fn next_scale(&mut self, target: f32, now: Instant) -> f32 {
        if (self.current_scale - target).abs() <= f32::EPSILON {
            self.animation = None;
            return target;
        }
        let animation = match self.animation {
            Some(animation) if animation.target == target => animation,
            Some(_) | None => {
                let animation = ScaleAnimation {
                    from: self.current_scale,
                    started: now,
                    target,
                };
                self.animation = Some(animation);
                animation
            }
        };
        let full_travel = if target > animation.from {
            self.grow_duration
        } else {
            self.shrink_duration
        };
        let duration = full_travel.mul_f32(self.travelled_fraction(animation));
        let progress = (now
            .saturating_duration_since(animation.started)
            .as_secs_f32()
            / duration.as_secs_f32())
        .min(1.0);
        let eased = 1.0 - (1.0 - progress) * (1.0 - progress);
        animation.from + (target - animation.from) * eased
    }

    fn travelled_fraction(&self, animation: ScaleAnimation) -> f32 {
        let span = self.scale_factor - 1.0;
        if span <= f32::EPSILON {
            return 1.0;
        }
        ((animation.target - animation.from).abs() / span).clamp(0.0, 1.0)
    }
}

impl MotionSample {
    pub fn new(at: Instant, dx: i32, dy: i32) -> Self {
        Self { at, dx, dy }
    }
}

fn scale_changed(previous: f32, current: f32) -> Option<f32> {
    if (current - previous).abs() > f32::EPSILON {
        return Some(current);
    }
    None
}

fn scale_event(previous: f32, current: f32, shake: Option<f64>) -> Option<ScaleEvent> {
    let was_scaled = previous > 1.0 + f32::EPSILON;
    let is_scaled = current > 1.0 + f32::EPSILON;
    match (was_scaled, is_scaled) {
        (false, true) => Some(ScaleEvent::Grew {
            tortuosity: shake.unwrap_or(0.0),
        }),
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
            enabled: true,
            shake_strictness: 6.5,
            regrow_strictness: 2.5,
            shake_min_extent_px: 150,
            regrow_min_extent_px: 60,
            shake_window_ms: 1000,
            scale_factor: 4,
            calm_duration_ms: 100,
            grow_ms: 250,
            shrink_ms: 225,
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
            ("side-to-side rub triggers", wiggle(200, 125, 2000), true),
            ("vigorous shake triggers", wiggle(240, 64, 2000), true),
            ("big fast arm shake triggers", wiggle(700, 125, 2000), true),
            ("small wiggle never triggers", wiggle(90, 96, 3000), false),
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
                "vertical rub triggers too",
                wiggle(200, 125, 2000)
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
    fn vigorous_shake_triggers_within_a_second() {
        let mut detector = ShakeDetector::new(&config());
        let grew_at = feed(&mut detector, Instant::now(), &wiggle(240, 64, 2000));
        let at = grew_at.expect("vigorous shake must trigger");
        assert!(at <= 1000, "trigger should land within 1s, got {at}ms");
    }

    fn tick(detector: &mut ShakeDetector, t0: Instant, from_ms: u64, to_ms: u64) -> f32 {
        let mut scale = detector.current_scale;
        let mut ms = from_ms;
        while ms <= to_ms {
            let update = detector.record(MotionSample::new(t0 + Duration::from_millis(ms), 0, 0));
            if let Some(changed) = update.scale_changed {
                scale = changed;
            }
            ms += 16;
        }
        scale
    }

    #[test]
    fn grow_and_shrink_match_configured_durations() {
        let t0 = Instant::now();

        let mut grower = ShakeDetector::new(&config());
        grower.growing = true;
        let mid = tick(&mut grower, t0, 0, 176);
        assert!(mid < 4.0, "grow must not finish before 250ms, got {mid}");
        assert!(mid > 3.1, "grow must front-load via ease-out, got {mid}");
        let full = tick(&mut grower, t0, 192, 280);
        assert_eq!(full, 4.0, "grow must complete within 250ms, got {full}");

        let mut shrinker = ShakeDetector::new(&config());
        shrinker.growing = true;
        tick(&mut shrinker, t0, 0, 280);
        shrinker.growing = false;
        shrinker.last_shake = None;
        let mid = tick(&mut shrinker, t0, 296, 400);
        assert!(mid > 1.0, "shrink must not finish before 225ms, got {mid}");
        assert!(mid < 2.5, "shrink must front-load via ease-out, got {mid}");
        let done = tick(&mut shrinker, t0, 416, 640);
        assert_eq!(done, 1.0, "shrink must complete within 225ms, got {done}");
    }

    #[test]
    fn sustained_shakes_hold_full_scale_across_shake_speeds() {
        for half_period_ms in [48, 64, 96, 128] {
            let mut detector = ShakeDetector::new(&config());
            let t0 = Instant::now();
            let mut grown = false;
            let mut lowest = 4.0;
            for (ms, dx, dy) in wiggle(240, half_period_ms, 3000) {
                let at = t0 + Duration::from_millis(ms);
                detector.record(MotionSample::new(at, dx, dy));
                grown |= detector.current_scale >= 4.0;
                if grown {
                    lowest = detector.current_scale.min(lowest);
                }
            }
            assert!(
                grown,
                "half period {half_period_ms}ms must reach full scale"
            );
            assert_eq!(
                lowest, 4.0,
                "half period {half_period_ms}ms must not shrink mid-shake"
            );
        }
    }

    fn shake_then_shrink(detector: &mut ShakeDetector, t0: Instant, shrink_ticks: u32) -> u64 {
        let trace = wiggle(240, 64, 1500);
        for (ms, dx, dy) in &trace {
            detector.record(MotionSample::new(t0 + Duration::from_millis(*ms), *dx, *dy));
        }
        let mut ms = trace.last().expect("trace must not be empty").0;
        let mut shrinking = false;
        for _ in 0..shrink_ticks.max(1) {
            loop {
                ms += 16;
                let at = t0 + Duration::from_millis(ms);
                let shrank = detector.record(MotionSample::new(at, 0, 0)).scale_changed;
                if shrinking || shrank.is_some() {
                    shrinking = true;
                    break;
                }
            }
        }
        assert!(detector.current_scale < 4.0, "shrink must have begun");
        ms
    }

    #[test]
    fn resumed_shake_regrows_while_the_cursor_is_still_shrinking() {
        for shrink_ticks in [1, 4, 7] {
            let mut detector = ShakeDetector::new(&config());
            let t0 = Instant::now();
            let resume_at = shake_then_shrink(&mut detector, t0, shrink_ticks);
            let scale_at_resume = detector.current_scale;

            let mut regrown_at = None;
            for (ms, dx, dy) in wiggle(240, 64, 500) {
                let at = t0 + Duration::from_millis(ms + resume_at);
                detector.record(MotionSample::new(at, dx, dy));
                if detector.current_scale >= 4.0 {
                    regrown_at = Some(ms);
                    break;
                }
            }
            let regrown_at = regrown_at
                .unwrap_or_else(|| panic!("resumed shake must regrow from {scale_at_resume}"));
            assert!(
                regrown_at <= 350,
                "resumed shake from {scale_at_resume} must regrow promptly, took {regrown_at}ms"
            );
        }
    }

    #[test]
    fn straight_glide_while_shrinking_never_regrows() {
        for speed in [10, 25, 40, 80] {
            let mut detector = ShakeDetector::new(&config());
            let t0 = Instant::now();
            let mut ms = shake_then_shrink(&mut detector, t0, 1);
            let mut previous = detector.current_scale;
            for _ in 0..60 {
                ms += 16;
                detector.record(MotionSample::new(t0 + Duration::from_millis(ms), speed, 0));
                assert!(
                    detector.current_scale <= previous,
                    "glide at {speed}px/tick must not regrow the cursor"
                );
                previous = detector.current_scale;
            }
            assert_eq!(previous, 1.0, "glide at {speed}px/tick must fully restore");
        }
    }

    #[test]
    fn shrink_starts_one_calm_duration_after_the_shake_stops() {
        let mut detector = ShakeDetector::new(&config());
        let t0 = Instant::now();
        let trace = wiggle(240, 64, 2000);
        for (ms, dx, dy) in &trace {
            detector.record(MotionSample::new(t0 + Duration::from_millis(*ms), *dx, *dy));
        }
        assert_eq!(detector.current_scale, 4.0, "shake must reach full scale");
        let last_shake = detector.last_shake.expect("shake must be recorded");

        let mut ms = trace.last().expect("trace must not be empty").0;
        let shrink_at = loop {
            ms += 16;
            let at = t0 + Duration::from_millis(ms);
            assert!(at - last_shake < Duration::from_secs(1), "never shrank");
            if detector
                .record(MotionSample::new(at, 0, 0))
                .scale_changed
                .is_some()
            {
                break at - last_shake;
            }
        };
        assert!(
            shrink_at > Duration::from_millis(100) && shrink_at <= Duration::from_millis(132),
            "shrink must start one calm duration after the last shake, got {shrink_at:?}"
        );
    }

    #[test]
    fn sustained_shake_holds_growth_until_calm_then_restores() {
        let mut detector = ShakeDetector::new(&config());
        let t0 = Instant::now();
        let trace = wiggle(240, 64, 3000);
        let grew_at = feed(&mut detector, t0, &trace).expect("shake must trigger");
        for (ms, dx, dy) in trace.iter().filter(|(ms, _, _)| *ms > grew_at) {
            let at = t0 + Duration::from_millis(*ms);
            let update = detector.record(MotionSample::new(at, *dx, *dy));
            assert!(
                !matches!(update.event, Some(ScaleEvent::Restored)),
                "must not restore mid-shake at {ms}ms"
            );
        }
        let mut t = Duration::from_millis(3000);
        for _ in 0..300 {
            t += Duration::from_millis(16);
            let update = detector.record(MotionSample::new(t0 + t, 0, 0));
            if let Some(ScaleEvent::Restored) = update.event {
                return;
            }
        }
        panic!("cursor must restore after calm");
    }

    #[test]
    fn short_burst_regrows_while_shrinking_but_not_from_rest() {
        let mut detector = ShakeDetector::new(&config());
        let t0 = Instant::now();
        feed(&mut detector, t0, &wiggle(240, 64, 1200)).expect("initial grow");
        let mut t = Duration::from_millis(1200);
        let mut shrinking = false;
        for _ in 0..300 {
            t += Duration::from_millis(16);
            let update = detector.record(MotionSample::new(t0 + t, 0, 0));
            if let Some(ScaleEvent::Restored) = update.event {
                panic!("restored before the shrink phase was observed");
            }
            if update.scale_changed.is_some_and(|scale| scale < 4.0) && !detector.growing {
                shrinking = true;
                break;
            }
        }
        assert!(shrinking, "shrink phase must begin after calm");
        let offset = t.as_millis() as u64;
        let burst: Vec<(u64, i32, i32)> = wiggle(80, 32, 300)
            .iter()
            .map(|(ms, dx, dy)| (ms + offset, *dx, *dy))
            .collect();
        for (ms, dx, dy) in &burst {
            let update =
                detector.record(MotionSample::new(t0 + Duration::from_millis(*ms), *dx, *dy));
            assert!(
                !matches!(update.event, Some(ScaleEvent::Restored)),
                "burst must regrow before the cursor fully restores"
            );
        }
        assert!(
            detector.growing,
            "short burst while shrinking must regrow via the relaxed threshold"
        );

        let mut rested = ShakeDetector::new(&config());
        let grew = feed(&mut rested, Instant::now(), &wiggle(80, 32, 300));
        assert_eq!(grew, None, "same burst from rest must not trigger");
    }

    #[test]
    fn shake_after_restore_grows_again() {
        let mut detector = ShakeDetector::new(&config());
        let t0 = Instant::now();
        feed(&mut detector, t0, &wiggle(240, 64, 1200)).expect("first grow");
        let mut t = Duration::from_millis(1200);
        let mut restored = false;
        for _ in 0..300 {
            t += Duration::from_millis(16);
            let update = detector.record(MotionSample::new(t0 + t, 0, 0));
            if let Some(ScaleEvent::Restored) = update.event {
                restored = true;
                break;
            }
        }
        assert!(restored, "cursor must restore after calm");
        let offset = t.as_millis() as u64;
        let regrow: Vec<(u64, i32, i32)> = wiggle(240, 64, 2000)
            .iter()
            .map(|(ms, dx, dy)| (ms + offset, *dx, *dy))
            .collect();
        assert!(
            feed(&mut detector, t0, &regrow).is_some(),
            "renewed shake must grow again"
        );
    }
}
