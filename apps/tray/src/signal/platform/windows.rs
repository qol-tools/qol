use std::sync::{Condvar, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;
use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};

const EXIT_GRACE: Duration = Duration::from_millis(4500);

static SHUTDOWN_TX: Mutex<Option<broadcast::Sender<()>>> = Mutex::new(None);
static FINISHED: Mutex<bool> = Mutex::new(false);
static FINISHED_CHANGED: Condvar = Condvar::new();

pub(crate) struct SignalListener;

impl Drop for SignalListener {
    fn drop(&mut self) {
        set_finished(true);
        unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), FALSE) };
        *SHUTDOWN_TX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

pub(super) fn install_signal_handler(
    shutdown_tx: broadcast::Sender<()>,
) -> std::io::Result<SignalListener> {
    *SHUTDOWN_TX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(shutdown_tx);
    set_finished(false);
    if unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), TRUE) } == 0 {
        let error = std::io::Error::last_os_error();
        *SHUTDOWN_TX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        return Err(error);
    }
    Ok(SignalListener)
}

unsafe extern "system" fn console_ctrl_handler(event: u32) -> BOOL {
    let Some(name) = event_name(event) else {
        return FALSE;
    };
    let sender = SHUTDOWN_TX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(sender) = sender else {
        return FALSE;
    };
    log::info!("[lifecycle] graceful shutdown requested by console event {name}");
    crate::tray::platform::request_shutdown(&sender);
    if ends_process_on_return(event) {
        wait_until_finished(EXIT_GRACE);
    }
    TRUE
}

fn event_name(event: u32) -> Option<&'static str> {
    match event {
        CTRL_C_EVENT => Some("CTRL_C"),
        CTRL_BREAK_EVENT => Some("CTRL_BREAK"),
        CTRL_CLOSE_EVENT => Some("CTRL_CLOSE"),
        CTRL_LOGOFF_EVENT => Some("CTRL_LOGOFF"),
        CTRL_SHUTDOWN_EVENT => Some("CTRL_SHUTDOWN"),
        _ => None,
    }
}

fn ends_process_on_return(event: u32) -> bool {
    matches!(
        event,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
    )
}

fn set_finished(value: bool) {
    *FINISHED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
    FINISHED_CHANGED.notify_all();
}

fn wait_until_finished(timeout: Duration) {
    let finished = FINISHED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = FINISHED_CHANGED.wait_timeout_while(finished, timeout, |finished| !*finished);
}
