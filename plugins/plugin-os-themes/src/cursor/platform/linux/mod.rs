mod motion;
mod runtime;
mod scale;
mod x11;

use std::process::{Command, Stdio};

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
        let url = qol_conventions::settings_url(PLUGIN_ID);
        Command::new(OPENER)
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to open settings URL")?;
        Ok(())
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

const OPENER: &str = "xdg-open";
