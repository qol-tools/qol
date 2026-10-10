// Copyright 2022-2022 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::{
    collections::HashMap,
    ptr,
    sync::mpsc::{self, Receiver, Sender},
    thread::JoinHandle,
};

use keyboard_types::{Code, Modifiers};
use windows_sys::Win32::{
    Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND, LPARAM, LRESULT, WIN32_ERROR, WPARAM},
    UI::{
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            KillTimer, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage,
            CW_USEDEFAULT, MSG, WM_APP, WM_CLOSE, WM_DESTROY, WM_HOTKEY, WM_TIMER, WNDCLASSW,
            WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_OVERLAPPED,
        },
    },
};

use crate::{hotkey::HotKey, GlobalHotKeyEvent};

/// Wakes the pump thread to drain the request queue.
const WM_REQUEST: u32 = WM_APP;
const RELEASE_POLL_TIMER: usize = 1;
const RELEASE_POLL_INTERVAL_MS: u32 = 10;

enum Request {
    Register {
        hotkey: HotKey,
        mods: HOT_KEY_MODIFIERS,
        vk: VIRTUAL_KEY,
        reply: Sender<crate::Result<()>>,
    },
    Unregister {
        hotkey: HotKey,
        reply: Sender<crate::Result<()>>,
    },
}

/// RegisterHotKey binds to the calling thread's window, so one dedicated
/// thread owns the hidden window, pumps its messages and runs every
/// register and unregister.
pub struct GlobalHotKeyManager {
    hwnd: usize,
    requests: Sender<Request>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for GlobalHotKeyManager {
    fn drop(&mut self) {
        // WM_CLOSE destroys the window, WM_DESTROY posts WM_QUIT, the pump returns.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl GlobalHotKeyManager {
    pub fn new() -> crate::Result<Self> {
        let (requests, request_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("global-hotkey".into())
            .spawn(move || pump(request_rx, ready_tx))?;
        match ready_rx.recv() {
            Ok(Ok(hwnd)) => Ok(Self {
                hwnd,
                requests,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(thread_gone()),
        }
    }

    pub fn register(&self, hotkey: HotKey) -> crate::Result<()> {
        let mut mods = MOD_NOREPEAT;
        if hotkey.mods.contains(Modifiers::SHIFT) {
            mods |= MOD_SHIFT;
        }
        if hotkey.mods.intersects(Modifiers::SUPER | Modifiers::META) {
            mods |= MOD_WIN;
        }
        if hotkey.mods.contains(Modifiers::ALT) {
            mods |= MOD_ALT;
        }
        if hotkey.mods.contains(Modifiers::CONTROL) {
            mods |= MOD_CONTROL;
        }

        let Some(vk) = key_to_vk(&hotkey.key) else {
            return Err(crate::Error::FailedToRegister(format!(
                "Unknown VKCode for {}",
                hotkey.key
            )));
        };
        self.call(|reply| Request::Register {
            hotkey,
            mods,
            vk,
            reply,
        })
    }

    pub fn unregister(&self, hotkey: HotKey) -> crate::Result<()> {
        self.call(|reply| Request::Unregister { hotkey, reply })
    }

    pub fn register_all(&self, hotkeys: &[HotKey]) -> crate::Result<()> {
        for hotkey in hotkeys {
            self.register(*hotkey)?;
        }
        Ok(())
    }

    pub fn unregister_all(&self, hotkeys: &[HotKey]) -> crate::Result<()> {
        for hotkey in hotkeys {
            self.unregister(*hotkey)?;
        }
        Ok(())
    }

    fn call(
        &self,
        request: impl FnOnce(Sender<crate::Result<()>>) -> Request,
    ) -> crate::Result<()> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.requests
            .send(request(reply_tx))
            .map_err(|_| thread_gone())?;
        if unsafe { PostMessageW(self.hwnd as HWND, WM_REQUEST, 0, 0) } == 0 {
            return Err(crate::Error::OsError(std::io::Error::last_os_error()));
        }
        reply_rx.recv().map_err(|_| thread_gone())?
    }
}

fn thread_gone() -> crate::Error {
    crate::Error::OsError(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "the global hotkey thread exited",
    ))
}

fn pump(requests: Receiver<Request>, ready: Sender<crate::Result<usize>>) {
    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(hwnd as usize));

    // hotkey id -> virtual key still held down
    let mut held: HashMap<u32, u16> = HashMap::new();
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            match msg.message {
                WM_REQUEST => handle_requests(hwnd, &requests, &mut held),
                WM_HOTKEY => {
                    let vk = HIWORD(msg.lParam as u32);
                    press(hwnd, &mut held, msg.wParam as u32, vk);
                }
                WM_TIMER if msg.wParam == RELEASE_POLL_TIMER => poll_released(hwnd, &mut held),
                _ => {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        DestroyWindow(hwnd);
    }
}

fn create_window() -> crate::Result<HWND> {
    let class_name = encode_wide("global_hotkey_app");
    unsafe {
        let hinstance = get_instance_handle();

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(global_hotkey_proc),
            lpszClassName: class_name.as_ptr(),
            hInstance: hinstance,
            ..std::mem::zeroed()
        };

        RegisterClassW(&wnd_class);

        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED |
            // WS_EX_TOOLWINDOW prevents this window from ever showing up in the taskbar, which
            // we want to avoid. If you remove this style, this window won't show up in the
            // taskbar *initially*, but it can show up at some later point. This can sometimes
            // happen on its own after several hours have passed, although this has proven
            // difficult to reproduce. Alternatively, it can be manually triggered by killing
            // `explorer.exe` and then starting the process back up.
            // It is unclear why the bug is triggered by waiting for several hours.
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            ptr::null(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            0,
            CW_USEDEFAULT,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            hinstance,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return Err(crate::Error::OsError(std::io::Error::last_os_error()));
        }
        Ok(hwnd)
    }
}

fn handle_requests(hwnd: HWND, requests: &Receiver<Request>, held: &mut HashMap<u32, u16>) {
    while let Ok(request) = requests.try_recv() {
        match request {
            Request::Register {
                hotkey,
                mods,
                vk,
                reply,
            } => {
                let _ = reply.send(register_on_thread(hwnd, hotkey, mods, vk));
            }
            Request::Unregister { hotkey, reply } => {
                let result = unsafe { UnregisterHotKey(hwnd, hotkey.id() as _) };
                if result == 0 {
                    let _ = reply.send(Err(crate::Error::FailedToUnRegister(hotkey)));
                    continue;
                }
                if held.remove(&hotkey.id()).is_some() {
                    send_released(hotkey.id());
                }
                let _ = reply.send(Ok(()));
            }
        }
    }
}

fn register_on_thread(
    hwnd: HWND,
    hotkey: HotKey,
    mods: HOT_KEY_MODIFIERS,
    vk: VIRTUAL_KEY,
) -> crate::Result<()> {
    let result = unsafe { RegisterHotKey(hwnd, hotkey.id() as _, mods, vk as _) };
    if result != 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(raw_os_error) => {
            let win32error = WIN32_ERROR::try_from(raw_os_error);
            if let Ok(ERROR_HOTKEY_ALREADY_REGISTERED) = win32error {
                Err(crate::Error::AlreadyRegistered(hotkey))
            } else {
                Err(crate::Error::OsError(error))
            }
        }
        _ => Err(crate::Error::OsError(error)),
    }
}

fn press(hwnd: HWND, held: &mut HashMap<u32, u16>, id: u32, vk: u16) {
    GlobalHotKeyEvent::send(GlobalHotKeyEvent {
        id,
        state: crate::HotKeyState::Pressed,
    });
    held.insert(id, vk);
    unsafe { SetTimer(hwnd, RELEASE_POLL_TIMER, RELEASE_POLL_INTERVAL_MS, None) };
}

fn poll_released(hwnd: HWND, held: &mut HashMap<u32, u16>) {
    held.retain(|id, vk| {
        let down = unsafe { GetAsyncKeyState(*vk as i32) } as u16 & 0x8000 != 0;
        if !down {
            send_released(*id);
        }
        down
    });
    if held.is_empty() {
        unsafe { KillTimer(hwnd, RELEASE_POLL_TIMER) };
    }
}

fn send_released(id: u32) {
    GlobalHotKeyEvent::send(GlobalHotKeyEvent {
        id,
        state: crate::HotKeyState::Released,
    });
}

unsafe extern "system" fn global_hotkey_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[inline(always)]
#[allow(non_snake_case)]
const fn HIWORD(x: u32) -> u16 {
    ((x >> 16) & 0xFFFF) as u16
}

pub fn encode_wide<S: AsRef<std::ffi::OsStr>>(string: S) -> Vec<u16> {
    std::os::windows::prelude::OsStrExt::encode_wide(string.as_ref())
        .chain(std::iter::once(0))
        .collect()
}

pub fn get_instance_handle() -> windows_sys::Win32::Foundation::HMODULE {
    // Gets the instance handle by taking the address of the
    // pseudo-variable created by the microsoft linker:
    // https://devblogs.microsoft.com/oldnewthing/20041025-00/?p=37483

    // This is preferred over GetModuleHandle(NULL) because it also works in DLLs:
    // https://stackoverflow.com/questions/21718027/getmodulehandlenull-vs-hinstance

    extern "C" {
        static __ImageBase: windows_sys::Win32::System::SystemServices::IMAGE_DOS_HEADER;
    }

    unsafe { &__ImageBase as *const _ as _ }
}

// used to build accelerators table from Key
fn key_to_vk(key: &Code) -> Option<VIRTUAL_KEY> {
    Some(match key {
        Code::KeyA => VK_A,
        Code::KeyB => VK_B,
        Code::KeyC => VK_C,
        Code::KeyD => VK_D,
        Code::KeyE => VK_E,
        Code::KeyF => VK_F,
        Code::KeyG => VK_G,
        Code::KeyH => VK_H,
        Code::KeyI => VK_I,
        Code::KeyJ => VK_J,
        Code::KeyK => VK_K,
        Code::KeyL => VK_L,
        Code::KeyM => VK_M,
        Code::KeyN => VK_N,
        Code::KeyO => VK_O,
        Code::KeyP => VK_P,
        Code::KeyQ => VK_Q,
        Code::KeyR => VK_R,
        Code::KeyS => VK_S,
        Code::KeyT => VK_T,
        Code::KeyU => VK_U,
        Code::KeyV => VK_V,
        Code::KeyW => VK_W,
        Code::KeyX => VK_X,
        Code::KeyY => VK_Y,
        Code::KeyZ => VK_Z,
        Code::Digit0 => VK_0,
        Code::Digit1 => VK_1,
        Code::Digit2 => VK_2,
        Code::Digit3 => VK_3,
        Code::Digit4 => VK_4,
        Code::Digit5 => VK_5,
        Code::Digit6 => VK_6,
        Code::Digit7 => VK_7,
        Code::Digit8 => VK_8,
        Code::Digit9 => VK_9,
        Code::Equal => VK_OEM_PLUS,
        Code::Comma => VK_OEM_COMMA,
        Code::Minus => VK_OEM_MINUS,
        Code::Period => VK_OEM_PERIOD,
        Code::Semicolon => VK_OEM_1,
        Code::Slash => VK_OEM_2,
        Code::Backquote => VK_OEM_3,
        Code::BracketLeft => VK_OEM_4,
        Code::Backslash => VK_OEM_5,
        Code::BracketRight => VK_OEM_6,
        Code::Quote => VK_OEM_7,
        Code::Backspace => VK_BACK,
        Code::Tab => VK_TAB,
        Code::Space => VK_SPACE,
        Code::Enter => VK_RETURN,
        Code::CapsLock => VK_CAPITAL,
        Code::Escape => VK_ESCAPE,
        Code::PageUp => VK_PRIOR,
        Code::PageDown => VK_NEXT,
        Code::End => VK_END,
        Code::Home => VK_HOME,
        Code::ArrowLeft => VK_LEFT,
        Code::ArrowUp => VK_UP,
        Code::ArrowRight => VK_RIGHT,
        Code::ArrowDown => VK_DOWN,
        Code::PrintScreen => VK_SNAPSHOT,
        Code::Insert => VK_INSERT,
        Code::Delete => VK_DELETE,
        Code::F1 => VK_F1,
        Code::F2 => VK_F2,
        Code::F3 => VK_F3,
        Code::F4 => VK_F4,
        Code::F5 => VK_F5,
        Code::F6 => VK_F6,
        Code::F7 => VK_F7,
        Code::F8 => VK_F8,
        Code::F9 => VK_F9,
        Code::F10 => VK_F10,
        Code::F11 => VK_F11,
        Code::F12 => VK_F12,
        Code::F13 => VK_F13,
        Code::F14 => VK_F14,
        Code::F15 => VK_F15,
        Code::F16 => VK_F16,
        Code::F17 => VK_F17,
        Code::F18 => VK_F18,
        Code::F19 => VK_F19,
        Code::F20 => VK_F20,
        Code::F21 => VK_F21,
        Code::F22 => VK_F22,
        Code::F23 => VK_F23,
        Code::F24 => VK_F24,
        Code::NumLock => VK_NUMLOCK,
        Code::Numpad0 => VK_NUMPAD0,
        Code::Numpad1 => VK_NUMPAD1,
        Code::Numpad2 => VK_NUMPAD2,
        Code::Numpad3 => VK_NUMPAD3,
        Code::Numpad4 => VK_NUMPAD4,
        Code::Numpad5 => VK_NUMPAD5,
        Code::Numpad6 => VK_NUMPAD6,
        Code::Numpad7 => VK_NUMPAD7,
        Code::Numpad8 => VK_NUMPAD8,
        Code::Numpad9 => VK_NUMPAD9,
        Code::NumpadAdd => VK_ADD,
        Code::NumpadDecimal => VK_DECIMAL,
        Code::NumpadDivide => VK_DIVIDE,
        Code::NumpadEnter => VK_RETURN,
        Code::NumpadEqual => VK_E,
        Code::NumpadMultiply => VK_MULTIPLY,
        Code::NumpadSubtract => VK_SUBTRACT,
        Code::ScrollLock => VK_SCROLL,
        Code::AudioVolumeDown => VK_VOLUME_DOWN,
        Code::AudioVolumeUp => VK_VOLUME_UP,
        Code::AudioVolumeMute => VK_VOLUME_MUTE,
        Code::MediaPlay => VK_PLAY,
        Code::MediaPause => VK_PAUSE,
        Code::MediaPlayPause => VK_MEDIA_PLAY_PAUSE,
        Code::MediaStop => VK_MEDIA_STOP,
        Code::MediaTrackNext => VK_MEDIA_NEXT_TRACK,
        Code::MediaTrackPrevious => VK_MEDIA_PREV_TRACK,
        Code::Pause => VK_PAUSE,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const DROP_TIMEOUT: Duration = Duration::from_secs(10);

    #[test]
    fn hotkeys_register_once_across_threads_and_the_manager_drops_promptly() {
        let manager = GlobalHotKeyManager::new().unwrap();
        let hotkey = HotKey::new(
            Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
            Code::F24,
        );

        manager.register(hotkey).unwrap();
        match manager.register(hotkey) {
            Err(crate::Error::AlreadyRegistered(registered)) => assert_eq!(registered, hotkey),
            other => panic!("expected AlreadyRegistered, got {other:?}"),
        }
        manager.unregister(hotkey).unwrap();
        std::thread::scope(|scope| {
            scope
                .spawn(|| manager.register(hotkey))
                .join()
                .unwrap()
                .unwrap();
        });
        manager.unregister(hotkey).unwrap();

        let (dropped_tx, dropped_rx) = mpsc::channel();
        std::thread::spawn(move || {
            drop(manager);
            let _ = dropped_tx.send(());
        });
        assert!(
            dropped_rx.recv_timeout(DROP_TIMEOUT).is_ok(),
            "dropping the manager must stop its message pump"
        );
    }
}
