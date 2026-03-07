use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;

const VELOCITY_WINDOW: Duration = Duration::from_millis(150);

pub struct ShakeDetector {
    samples: VecDeque<MotionSample>,
    thresholds: Thresholds,
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
    pub should_reapply: bool,
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
    Grew { velocity: f64 },
    Restored,
}

struct Thresholds {
    velocity: f64,
    shakiness: f64,
    regrow_velocity: f64,
    regrow_shakiness: f64,
    post_trigger: f64,
}

impl ShakeDetector {
    pub fn new(config: &Config) -> Self {
        let scale_factor = config.scale_factor as f32;
        Self {
            samples: VecDeque::new(),
            thresholds: Thresholds::from(config),
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
        self.samples.push_back(sample);
        self.trim(sample.at);
        let metrics = motion_metrics(&self.samples);
        self.update(sample.at, metrics)
    }

    fn trim(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|sample| now - sample.at > VELOCITY_WINDOW)
        {
            self.samples.pop_front();
        }
    }

    fn update(&mut self, now: Instant, metrics: MotionMetrics) -> ScaleUpdate {
        let is_shake = self.is_shake(metrics.velocity, metrics.shakiness);
        if is_shake {
            self.growing = true;
            self.last_shake = Some(now);
        } else {
            self.maybe_stop_growing(now, metrics.velocity);
        }

        let previous_scale = self.current_scale;
        let target_scale = if self.growing { self.scale_factor } else { 1.0 };
        let next_scale = self.next_scale(target_scale);
        self.current_scale = next_scale;

        let scale_changed = scale_changed(previous_scale, next_scale);
        ScaleUpdate {
            scale_changed,
            should_reapply: scale_changed.is_none() && self.is_scaled(),
            event: scale_event(previous_scale, next_scale, metrics.velocity),
        }
    }

    fn is_shake(&self, velocity: f64, shakiness: f64) -> bool {
        if !self.growing && self.is_scaled() {
            return velocity > self.thresholds.regrow_velocity
                && shakiness > self.thresholds.regrow_shakiness;
        }
        velocity > self.thresholds.velocity && shakiness > self.thresholds.shakiness
    }

    fn maybe_stop_growing(&mut self, now: Instant, velocity: f64) {
        if !self.growing || self.current_scale < self.scale_factor - f32::EPSILON {
            return;
        }
        if velocity > self.thresholds.post_trigger {
            self.last_shake = Some(now);
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
            velocity: config.velocity_threshold,
            shakiness: config.shakiness_threshold,
            regrow_velocity: config.regrow_velocity_threshold,
            regrow_shakiness: config.regrow_shakiness_threshold,
            post_trigger: config.post_trigger_threshold,
        }
    }
}

struct MotionMetrics {
    velocity: f64,
    shakiness: f64,
}

fn motion_metrics(samples: &VecDeque<MotionSample>) -> MotionMetrics {
    MotionMetrics {
        velocity: velocity(samples),
        shakiness: shakiness(samples),
    }
}

fn scale_changed(previous: f32, current: f32) -> Option<f32> {
    if (current - previous).abs() > f32::EPSILON {
        return Some(current);
    }
    None
}

fn scale_event(previous: f32, current: f32, velocity: f64) -> Option<ScaleEvent> {
    let was_scaled = previous > 1.0 + f32::EPSILON;
    let is_scaled = current > 1.0 + f32::EPSILON;
    match (was_scaled, is_scaled) {
        (false, true) => Some(ScaleEvent::Grew { velocity }),
        (true, false) => Some(ScaleEvent::Restored),
        _ => None,
    }
}

fn velocity(samples: &VecDeque<MotionSample>) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let distance: f64 = samples
        .iter()
        .map(|sample| ((sample.dx as f64).powi(2) + (sample.dy as f64).powi(2)).sqrt())
        .sum();
    let elapsed = (samples.back().unwrap().at - samples.front().unwrap().at).as_secs_f64();
    if elapsed < f64::EPSILON {
        return 0.0;
    }
    distance / elapsed
}

fn shakiness(samples: &VecDeque<MotionSample>) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let total: f64 = samples
        .iter()
        .map(|sample| ((sample.dx as f64).powi(2) + (sample.dy as f64).powi(2)).sqrt())
        .sum();
    if total < 1.0 {
        return 0.0;
    }
    let net_x: f64 = samples.iter().map(|sample| sample.dx as f64).sum();
    let net_y: f64 = samples.iter().map(|sample| sample.dy as f64).sum();
    let net_distance = (net_x.powi(2) + net_y.powi(2)).sqrt();
    total / (net_distance + 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn velocity_empty_returns_zero() {
        assert_eq!(velocity(&VecDeque::new()), 0.0);
    }

    #[test]
    fn velocity_single_sample_returns_zero() {
        let mut samples = VecDeque::new();
        samples.push_back(MotionSample::new(Instant::now(), 100, 0));
        assert_eq!(velocity(&samples), 0.0);
    }

    #[test]
    fn velocity_300px_over_100ms_is_3000px_per_sec() {
        let mut samples = VecDeque::new();
        let t0 = Instant::now();
        samples.push_back(MotionSample::new(t0, 300, 0));
        samples.push_back(MotionSample::new(t0 + Duration::from_millis(100), 0, 0));
        let velocity = velocity(&samples);
        assert!((velocity - 3000.0).abs() < 1.0, "expected ~3000 px/s, got {velocity}");
    }

    #[test]
    fn shakiness_glide_is_low() {
        let mut samples = VecDeque::new();
        let t0 = Instant::now();
        for i in 0..9 {
            samples.push_back(MotionSample::new(t0 + Duration::from_millis(i * 16), 100, 0));
        }
        assert!(shakiness(&samples) < 1.5, "straight glide should have low shakiness");
    }

    #[test]
    fn shakiness_back_and_forth_is_high() {
        let mut samples = VecDeque::new();
        let t0 = Instant::now();
        for i in 0..9 {
            let dx = if i % 2 == 0 { 100 } else { -100 };
            samples.push_back(MotionSample::new(t0 + Duration::from_millis(i * 16), dx, 0));
        }
        assert!(shakiness(&samples) > 3.0, "back-and-forth should have high shakiness");
    }
}
