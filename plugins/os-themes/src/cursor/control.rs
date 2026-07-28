use std::sync::atomic::{AtomicBool, Ordering};

use super::{CursorPlatform, Platform};

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
        Platform.reset_external_stop();
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
        Platform.external_stop_requested() || !self.running.load(Ordering::Relaxed)
    }
}
