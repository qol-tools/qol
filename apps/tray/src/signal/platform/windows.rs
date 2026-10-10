use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use tokio::sync::broadcast;
use windows_sys::core::w;
use windows_sys::Win32::Foundation::{BOOL, FALSE, HWND, LPARAM, LRESULT, TRUE, WPARAM};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostThreadMessageW,
    RegisterClassW, TranslateMessage, MSG, WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION, WM_QUIT,
    WNDCLASSW,
};

const EXIT_GRACE: Duration = Duration::from_millis(4500);
const SESSION_WINDOW_CLASS: windows_sys::core::PCWSTR = w!("qol-tray-session");

static SHUTDOWN_TX: Mutex<Option<broadcast::Sender<()>>> = Mutex::new(None);
static FINISHED: Mutex<bool> = Mutex::new(false);
static FINISHED_CHANGED: Condvar = Condvar::new();
static SESSION_THREAD: AtomicU32 = AtomicU32::new(0);

pub(crate) struct SignalListener;

impl Drop for SignalListener {
    fn drop(&mut self) {
        set_finished(true);
        unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), FALSE) };
        let session_thread = SESSION_THREAD.swap(0, Ordering::SeqCst);
        if session_thread != 0 {
            unsafe { PostThreadMessageW(session_thread, WM_QUIT, 0, 0) };
        }
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
    let session_window = std::thread::Builder::new()
        .name("qol-session-end".to_string())
        .spawn(run_session_window);
    if let Err(error) = session_window {
        log::warn!("[lifecycle] session end listener thread failed: {error}");
    }
    Ok(SignalListener)
}

unsafe extern "system" fn console_ctrl_handler(event: u32) -> BOOL {
    let Some(name) = event_name(event) else {
        return FALSE;
    };
    if !request_shutdown("console event", name) {
        return FALSE;
    }
    if ends_process_on_return(event) {
        wait_until_finished(EXIT_GRACE);
    }
    TRUE
}

fn request_shutdown(source: &str, name: &str) -> bool {
    let sender = SHUTDOWN_TX
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Some(sender) = sender else {
        return false;
    };
    log::info!("[lifecycle] graceful shutdown requested by {source} {name}");
    crate::tray::platform::request_shutdown(&sender);
    true
}

fn run_session_window() {
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(session_window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: SESSION_WINDOW_CLASS,
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        log::warn!(
            "[lifecycle] session end window class failed: {}",
            std::io::Error::last_os_error()
        );
        return;
    }
    let window = unsafe {
        CreateWindowExW(
            0,
            SESSION_WINDOW_CLASS,
            SESSION_WINDOW_CLASS,
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if window.is_null() {
        log::warn!(
            "[lifecycle] session end window failed: {}",
            std::io::Error::last_os_error()
        );
        return;
    }
    SESSION_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn session_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match session_action(message, wparam) {
        SessionAction::AllowEnd => 1,
        SessionAction::Shutdown(name) => {
            request_shutdown("window message", name);
            0
        }
        SessionAction::ShutdownAndWait(name) => {
            if request_shutdown("window message", name) {
                wait_until_finished(EXIT_GRACE);
            }
            0
        }
        SessionAction::Handled => 0,
        SessionAction::Default => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SessionAction {
    AllowEnd,
    Shutdown(&'static str),
    ShutdownAndWait(&'static str),
    Handled,
    Default,
}

fn session_action(message: u32, wparam: WPARAM) -> SessionAction {
    match message {
        WM_QUERYENDSESSION => SessionAction::AllowEnd,
        WM_ENDSESSION if wparam != 0 => SessionAction::ShutdownAndWait("WM_ENDSESSION"),
        WM_ENDSESSION => SessionAction::Handled,
        WM_CLOSE => SessionAction::Shutdown("WM_CLOSE"),
        _ => SessionAction::Default,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_messages_map_to_shutdown_actions() {
        let cases = [
            (WM_QUERYENDSESSION, 0, SessionAction::AllowEnd),
            (WM_QUERYENDSESSION, 1, SessionAction::AllowEnd),
            (
                WM_ENDSESSION,
                1,
                SessionAction::ShutdownAndWait("WM_ENDSESSION"),
            ),
            (WM_ENDSESSION, 0, SessionAction::Handled),
            (WM_CLOSE, 0, SessionAction::Shutdown("WM_CLOSE")),
            (WM_QUIT, 0, SessionAction::Default),
        ];
        for (message, wparam, expected) in cases {
            assert_eq!(session_action(message, wparam), expected);
        }
    }
}
