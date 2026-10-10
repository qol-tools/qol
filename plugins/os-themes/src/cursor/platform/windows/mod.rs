mod focus;
mod raster;
mod session;

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};

use crate::config::Config;
use crate::cursor::CursorEffect;

use super::shake::{self, ShakeBackend};
use super::CursorPlatform;

pub struct Platform;

static EXTERNAL_STOP: AtomicBool = AtomicBool::new(false);

impl CursorPlatform for Platform {
    fn create_effect(&self) -> Box<dyn CursorEffect> {
        shake::create_effect(WindowsBackend)
    }

    fn install_signal_handlers(&self) {
        if unsafe { SetConsoleCtrlHandler(Some(handle_control), TRUE) } == 0 {
            log::warn!(
                "console stop handler unavailable: {}",
                std::io::Error::last_os_error()
            );
        }
        let listener = std::thread::Builder::new()
            .name("os-themes-stop".into())
            .spawn(|| match qol_process::wait_for_stop_request() {
                Ok(()) => EXTERNAL_STOP.store(true, Ordering::Relaxed),
                Err(error) => log::warn!("stop request listener unavailable: {error}"),
            });
        if let Err(error) = listener {
            log::warn!("stop request listener thread failed: {error}");
        }
    }

    fn reset_external_stop(&self) {
        EXTERNAL_STOP.store(false, Ordering::SeqCst);
    }

    fn external_stop_requested(&self) -> bool {
        EXTERNAL_STOP.load(Ordering::Relaxed)
    }

    fn recover(&self) {
        session::reload_system_cursors();
    }
}

struct WindowsBackend;

impl ShakeBackend for WindowsBackend {
    type Session = session::CursorSession;
    type Focus = focus::GameFocusDetector;

    fn open_session(&self, config: &Config) -> Result<session::CursorSession> {
        qol_windowing::platform::windows::ensure_dpi_awareness();
        log::info!("started mode=system-cursors");
        Ok(session::CursorSession::open(config.scale_factor))
    }

    fn open_focus(&self) -> Result<focus::GameFocusDetector> {
        qol_windowing::platform::windows::ensure_dpi_awareness();
        Ok(focus::GameFocusDetector)
    }
}

unsafe extern "system" fn handle_control(control: u32) -> BOOL {
    match control {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            EXTERNAL_STOP.store(true, Ordering::Relaxed);
            TRUE
        }
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            EXTERNAL_STOP.store(true, Ordering::Relaxed);
            session::reload_system_cursors();
            TRUE
        }
        _ => FALSE,
    }
}
