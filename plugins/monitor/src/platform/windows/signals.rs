use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};

use crate::daemon::Command;

const EXIT_GRACE: Duration = Duration::from_millis(4500);
const EXIT_POLL: Duration = Duration::from_millis(50);

static SENDER: Mutex<Option<Sender<Command>>> = Mutex::new(None);
static FINISHED: AtomicBool = AtomicBool::new(false);

pub(crate) struct SignalGuard;

impl Drop for SignalGuard {
    fn drop(&mut self) {
        FINISHED.store(true, Ordering::Release);
        SENDER.lock().unwrap_or_else(PoisonError::into_inner).take();
        unsafe {
            SetConsoleCtrlHandler(Some(handle_control), FALSE);
        }
    }
}

pub(crate) fn install_signal_handlers(tx: Sender<Command>) -> SignalGuard {
    FINISHED.store(false, Ordering::Release);
    *SENDER.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
    if unsafe { SetConsoleCtrlHandler(Some(handle_control), TRUE) } == 0 {
        log::warn!(
            "cannot register the console control handler: {}",
            std::io::Error::last_os_error()
        );
    }
    SignalGuard
}

unsafe extern "system" fn handle_control(control: u32) -> BOOL {
    let ends_session = matches!(
        control,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
    );
    if !ends_session && !matches!(control, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        return FALSE;
    }
    let sender = SENDER.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(tx) = sender {
        let _ = tx.send(Command::Kill);
    }
    if ends_session {
        let deadline = Instant::now() + EXIT_GRACE;
        while !FINISHED.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(EXIT_POLL);
        }
    }
    TRUE
}
