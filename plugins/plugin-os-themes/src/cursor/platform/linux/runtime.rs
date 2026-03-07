'use std::time::{Duration, Instant};

use anyhow::Result;

use crate::config::Config;
use crate::cursor::{CursorEffect, RunControl};

use super::motion::{MotionSample, ScaleEvent, ScaleUpdate, ShakeDetector};
use super::x11::CursorSession;

const POLL_INTERVAL: Duration = Duration::from_millis(16);

pub fn create_effect() -> Box<dyn CursorEffect> {
    Box::new(LinuxCursorEffect)
}

struct LinuxCursorEffect;

impl CursorEffect for LinuxCursorEffect {
    fn run(&self, config: &Config, control: &dyn RunControl) -> Result<()> {
        let Some(mut session) = open_session(config.scale_factor)? else {
            return Ok(());
        };
        let mut runner = EffectRunner::new(config, session.pointer_position());
        run_loop(control, &mut session, &mut runner);
        session.restore();
        Ok(())
    }
}

fn open_session(scale_factor: u32) -> Result<Option<CursorSession>> {
    let Some(session) = CursorSession::open(scale_factor)? else {
        eprintln!("[shake-to-grow] warn: failed to load base cursor pixels");
        return Ok(None);
    };
    eprintln!("[shake-to-grow] started");
    Ok(Some(session))
}

fn run_loop(control: &dyn RunControl, session: &mut CursorSession, runner: &mut EffectRunner) {
    while !control.should_stop() {
        std::thread::sleep(POLL_INTERVAL);
        let update = runner.tick(Instant::now(), session.pointer_position());
        apply_update(session, update);
    }
}

fn apply_update(session: &mut CursorSession, update: ScaleUpdate) {
    if let Some(event) = update.event {
        log_event(event);
    }
    if let Some(scale) = update.scale_changed {
        session.set_scale(scale);
        return;
    }
    if update.should_reapply {
        session.reapply_active();
    }
}

struct EffectRunner {
    detector: ShakeDetector,
    last_position: (i32, i32),
}

impl EffectRunner {
    fn new(config: &Config, position: (i32, i32)) -> Self {
        Self {
            detector: ShakeDetector::new(config),
            last_position: position,
        }
    }

    fn tick(&mut self, now: Instant, position: (i32, i32)) -> ScaleUpdate {
        let dx = position.0 - self.last_position.0;
        let dy = position.1 - self.last_position.1;
        self.last_position = position;
        self.detector.record(MotionSample::new(now, dx, dy))
    }
}

fn log_event(event: ScaleEvent) {
    match event {
        ScaleEvent::Grew { velocity } => eprintln!("[shake-to-grow] grow velocity={velocity:.0} px/s"),
        ScaleEvent::Restored => eprintln!("[shake-to-grow] restore"),
    }
}
