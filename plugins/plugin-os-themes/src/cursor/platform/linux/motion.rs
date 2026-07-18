use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;

pub struct ShakeDetector {
    trail: Trail,
    window: Duration,
    strictness: f64,
    regrow_strictness: f64,
    min_extent: f64,
    regrow_min_extent: f64,
    calm_duration: Duration,
    scale_factor: f32,
    grow_rate: f32,
    shrink_rate: f32,
    current_scale: f32,
    growing: bool,
    last_shake: Option<Instant>,
    last_tick: Option<Instant>,
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

    fn advance(&mut self, sample: MotionSample, window: Duration) -> Option<TrailShape> {
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
            return None;
        }
        self.vertices.push_back(vertex);
        self.shape()
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
            grow_rate: rate_per_second(scale_factor, config.grow_ms),
            shrink_rate: rate_per_second(scale_factor, config.shrink_ms),
            current_scale: 1.0,
            growing: false,
            last_shake: None,
            last_tick: None,
        }
    }

    pub fn record(&mut self, sample: MotionSample) -> ScaleUpdate {
        let dt = self
            .last_tick
            .map_or(Duration::ZERO, |last| {
                sample.at.saturating_duration_since(last)
            })
            .min(Duration::from_millis(50));
        self.last_tick = Some(sample.at);
        let shake = self.detect(sample);
        self.update(sample.at, shake, dt)
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
        let shape = self.trail.advance(sample, self.window)?;
        if shape.extent < min_extent || shape.tortuosity <= strictness {
            return None;
        }
        Some(shape.tortuosity)
    }

    fn is_scaled(&self) -> bool {
        self.current_scale > 1.0 + f32::EPSILON
    }

    fn update(&mut self, now: Instant, shake: Option<f64>, dt: Duration) -> ScaleUpdate {
        if shake.is_some() {
            self.growing = true;
            self.last_shake = Some(now);
        } else {
            self.maybe_stop_growing(now);
        }

        let previous_scale = self.current_scale;
        let target_scale = if self.growing { self.scale_factor } else { 1.0 };
        let next_scale = self.next_scale(target_scale, dt);
        self.current_scale = next_scale;

        ScaleUpdate {
            scale_changed: scale_changed(previous_scale, next_scale),
            event: scale_event(previous_scale, next_scale, shake),
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
            self.trail.reset();
        }
    }

    fn next_scale(&self, target: f32, dt: Duration) -> f32 {
        let dt_secs = dt.as_secs_f32();
        if target > self.current_scale {
            return (self.current_scale + self.grow_rate * dt_secs).min(target);
        }
        if target < self.current_scale {
            return (self.current_scale - self.shrink_rate * dt_secs).max(target);
        }
        self.current_scale
    }
}

fn rate_per_second(scale_factor: f32, duration_ms: u32) -> f32 {
    (scale_factor - 1.0) * 1000.0 / duration_ms.max(1) as f32
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
            regrow_strictness: 3.0,
            shake_min_extent_px: 150,
            regrow_min_extent_px: 60,
            shake_window_ms: 1000,
            scale_factor: 4,
            calm_duration_ms: 650,
            grow_ms: 110,
            shrink_ms: 300,
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
        let mid = tick(&mut grower, t0, 0, 80);
        assert!(mid < 4.0, "grow must not finish before 110ms, got {mid}");
        let full = tick(&mut grower, t0, 96, 160);
        assert_eq!(full, 4.0, "grow must complete within 110ms, got {full}");

        let mut shrinker = ShakeDetector::new(&config());
        shrinker.growing = true;
        tick(&mut shrinker, t0, 0, 200);
        shrinker.growing = false;
        shrinker.last_shake = None;
        let mid = tick(&mut shrinker, t0, 216, 440);
        assert!(mid > 1.0, "shrink must not finish before 300ms, got {mid}");
        let done = tick(&mut shrinker, t0, 456, 700);
        assert_eq!(done, 1.0, "shrink must complete within 300ms, got {done}");
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
        let burst: Vec<(u64, i32, i32)> = wiggle(80, 64, 300)
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
        let grew = feed(&mut rested, Instant::now(), &wiggle(80, 64, 300));
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
