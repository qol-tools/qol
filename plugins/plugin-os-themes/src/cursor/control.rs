// Some symbols here are only consumed by the Linux cursor effect; the macOS
// and Windows stubs short-circuit before touching this control loop. The
// allow keeps clippy quiet on hosts that don't yet have a real impl.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};

pub trait RunControl {
    fn should_stop(&self) -> bool;
}

pub struct RunState {
    running: AtomicBool,
    reload_requested: AtomicBool,
}

impl RunState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            reload_requested: AtomicBool::new(false),
        }
    }

    pub fn reset(&self) {
        EXTERNAL_STOP.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
        self.reload_requested.store(false, Ordering::SeqCst);
    }

    pub fn request_shutdown(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn request_reload(&self) {
        self.reload_requested.store(true, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn reload_requested(&self) -> bool {
        self.reload_requested.load(Ordering::SeqCst)
    }
}

impl Default for RunState {
    fn default() -> Self {
        Self::new()
    }
}

impl RunControl for RunState {
    fn should_stop(&self) -> bool {
        EXTERNAL_STOP.load(Ordering::Relaxed) || !self.running.load(Ordering::Relaxed)
    }
}

static EXTERNAL_STOP: AtomicBool = AtomicBool::new(false);

pub(crate) fn request_external_stop() {
    EXTERNAL_STOP.store(true, Ordering::Relaxed);
}
