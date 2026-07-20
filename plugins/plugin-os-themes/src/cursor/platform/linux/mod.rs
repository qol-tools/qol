mod display;
mod motion;
mod runtime;
mod scale;

use anyhow::{Context, Result};

use crate::config::PLUGIN_ID;
use crate::cursor::control::request_external_stop;
use crate::cursor::CursorEffect;

use super::CursorPlatform;

pub struct Platform;

impl CursorPlatform for Platform {
    fn create_effect(&self) -> Box<dyn CursorEffect> {
        runtime::create_effect()
    }

    fn install_signal_handlers(&self) {
        register(libc::SIGTERM);
        register(libc::SIGINT);
    }

    fn open_settings(&self) -> Result<()> {
        qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
            .context("failed to open settings URL")
    }
}

fn register(signal: libc::c_int) {
    let handler: extern "C" fn(libc::c_int) = handle_signal;
    unsafe {
        libc::signal(signal, handler as libc::sighandler_t);
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    request_external_stop();
}
