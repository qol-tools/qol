mod display;
mod game_focus;
mod motion;
mod runtime;
mod scale;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::cursor::CursorEffect;

use super::CursorPlatform;

pub struct Platform;

static EXTERNAL_STOP: AtomicBool = AtomicBool::new(false);

impl CursorPlatform for Platform {
    fn create_effect(&self) -> Box<dyn CursorEffect> {
        runtime::create_effect()
    }

    fn install_signal_handlers(&self) {
        register(libc::SIGTERM);
        register(libc::SIGINT);
    }

    fn reset_external_stop(&self) {
        EXTERNAL_STOP.store(false, Ordering::SeqCst);
    }

    fn external_stop_requested(&self) -> bool {
        EXTERNAL_STOP.load(Ordering::Relaxed)
    }
}

fn register(signal: libc::c_int) {
    let handler: extern "C" fn(libc::c_int) = handle_signal;
    unsafe {
        libc::signal(signal, handler as libc::sighandler_t);
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    EXTERNAL_STOP.store(true, Ordering::Relaxed);
}
