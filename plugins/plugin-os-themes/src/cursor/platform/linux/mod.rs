mod motion;
mod runtime;
mod scale;
mod x11;

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::cursor::request_external_stop;

pub use runtime::create_effect;
pub fn install_signal_handlers() {
    register(libc::SIGTERM);
    register(libc::SIGINT);
}

pub fn open_settings() -> Result<()> {
    Command::new(SETTINGS_URL)
        .arg(PLUGIN_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
}

fn register(signal: libc::c_int) {
    unsafe {
        libc::signal(signal, handle_signal as libc::sighandler_t);
    }
}

extern "C" fn handle_signal(_: libc::c_int) {
    request_external_stop();
}

const SETTINGS_URL: &str = "xdg-open";
const PLUGIN_URL: &str = "http://127.0.0.1:42700/plugins/plugin-os-themes/";
