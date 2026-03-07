use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use qol_plugin_api::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_plugin_api::{PlatformStateClient, Subscription};

use crate::config::Config;
use crate::cursor::{CursorEffect, RunControl};

use super::motion::{MotionSample, ScaleEvent, ScaleUpdate, ShakeDetector};
use super::x11::CursorSession;

const TICK_INTERVAL: Duration = Duration::from_millis(16);

pub fn create_effect() -> Box<dyn CursorEffect> {
    Box::new(LinuxCursorEffect)
}

struct LinuxCursorEffect;

impl CursorEffect for LinuxCursorEffect {
    fn run(&self, config: &Config, control: &dyn RunControl) -> Result<()> {
        let Some(mut session) = open_session(config.shake_to_grow().scale_factor)? else {
            return Ok(());
        };
        let client = PlatformStateClient::from_env();
        let subscription = client
            .subscribe(vec![RuntimeEventKind::CursorMoved])
            .context("failed to subscribe to cursor events")?;
        let rx = spawn_reader(subscription);
        let mut detector = ShakeDetector::new(config);
        let mut last_pos: Option<(f32, f32)> = None;
        let mut scaled = false;
        loop {
            if control.should_stop() {
                break;
            }
            let timeout = if scaled { TICK_INTERVAL } else { Duration::from_secs(86400) };
            let sample = match rx.recv_timeout(timeout) {
                Ok((x, y)) => {
                    let (dx, dy) = delta(last_pos, x, y);
                    last_pos = Some((x, y));
                    MotionSample::new(Instant::now(), dx, dy)
                }
                Err(RecvTimeoutError::Timeout) => MotionSample::new(Instant::now(), 0, 0),
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let update = detector.record(sample);
            scaled = update.scale_changed.map_or(scaled, |s| s > 1.0 + f32::EPSILON);
            apply_update(&mut session, update);
        }
        session.restore();
        Ok(())
    }
}

fn spawn_reader(mut subscription: Subscription) -> mpsc::Receiver<(f32, f32)> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Some(event) = subscription.next_event() {
            let RuntimeEvent::CursorMoved { x, y } = event else {
                continue;
            };
            if tx.send((x, y)).is_err() {
                break;
            }
        }
    });
    rx
}

fn delta(last: Option<(f32, f32)>, x: f32, y: f32) -> (i32, i32) {
    let Some((lx, ly)) = last else {
        return (0, 0);
    };
    ((x - lx) as i32, (y - ly) as i32)
}

fn open_session(scale_factor: u32) -> Result<Option<CursorSession>> {
    let Some(session) = CursorSession::open(scale_factor)? else {
        eprintln!("[shake-to-grow] warn: failed to load base cursor pixels");
        return Ok(None);
    };
    eprintln!("[shake-to-grow] started");
    Ok(Some(session))
}

fn apply_update(session: &mut CursorSession, update: ScaleUpdate) {
    if let Some(event) = update.event {
        log_event(event);
    }
    if let Some(scale) = update.scale_changed {
        session.set_scale(scale);
    }
}

fn log_event(event: ScaleEvent) {
    match event {
        ScaleEvent::Grew { velocity } => {
            eprintln!("[shake-to-grow] grow velocity={velocity:.0} px/s")
        }
        ScaleEvent::Restored => eprintln!("[shake-to-grow] restore"),
    }
}
