use std::cell::RefCell;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{bail, Result};
use qol_runtime::keyremap_marker;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, MapVirtualKeyW, SendInput, ToUnicodeEx, HKL, INPUT, INPUT_0,
    INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    MOUSE_EVENT_FLAGS, VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_MENU, VK_PACKET, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use super::foreground::Foreground;
use super::keys;
use super::machine::{Decision, Event, Machine, Output};
use crate::platform::engine::remap::{modifiers_from_bits, MouseButton, ResolvedConfig};
use crate::platform::engine::Remapper;

const ALTGR_CONTROL_SCAN: u32 = 0x21D;
const NO_KEYBOARD_STATE_CHANGE: u32 = 0x4;
const READY_TIMEOUT: Duration = Duration::from_secs(2);

static CONFIG: OnceLock<RwLock<Arc<ResolvedConfig>>> = OnceLock::new();

thread_local! {
    static STATE: RefCell<HookState> = RefCell::new(HookState::default());
}

#[derive(Default)]
struct HookState {
    machine: Machine,
    foreground: Foreground,
}

enum Gesture {
    Button(MouseButton, bool),
    Wheel(bool, i32),
}

pub(super) struct Hooks {
    thread_id: u32,
    thread: JoinHandle<()>,
}

impl Hooks {
    pub(super) fn start(config: ResolvedConfig) -> Result<Self> {
        if CONFIG.set(RwLock::new(Arc::new(config))).is_err() {
            bail!("the Windows input hooks are already installed");
        }
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("keyremap-hooks".into())
            .spawn(move || run(ready_tx))?;
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(thread_id)) => {
                log::info!("Windows keyboard and mouse hooks installed");
                Ok(Self { thread_id, thread })
            }
            Ok(Err(message)) => bail!(message),
            Err(RecvTimeoutError::Timeout) => {
                bail!("the Windows input hooks did not report readiness")
            }
            Err(RecvTimeoutError::Disconnected) => {
                bail!("the Windows input hooks exited before reporting readiness")
            }
        }
    }
}

impl Remapper for Hooks {
    fn swap_config(&self, config: ResolvedConfig) {
        if let Some(lock) = CONFIG.get() {
            *lock
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(config);
        }
    }

    fn idle(&self) {}

    fn stop(self) {
        if unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) } == 0 {
            log::warn!(
                "could not stop the Windows input hooks: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        if self.thread.join().is_err() {
            log::warn!("the Windows input hook thread panicked while stopping");
        }
    }
}

fn run(ready: Sender<Result<u32, String>>) {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
    if keyboard.is_null() {
        let _ = ready.send(Err(format!(
            "failed to install the Windows keyboard hook: {}",
            std::io::Error::last_os_error()
        )));
        return;
    }
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
    if mouse.is_null() {
        let _ = ready.send(Err(format!(
            "failed to install the Windows mouse hook: {}",
            std::io::Error::last_os_error()
        )));
        unsafe { UnhookWindowsHookEx(keyboard) };
        return;
    }
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {}
    unsafe {
        UnhookWindowsHookEx(mouse);
        UnhookWindowsHookEx(keyboard);
    }
    let outputs = STATE.with(|state| state.borrow_mut().machine.release_all());
    send(&outputs);
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if std::panic::catch_unwind(|| on_key(wparam as u32, event)).unwrap_or(false) {
            return 1;
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        if std::panic::catch_unwind(|| on_mouse(wparam as u32, event)).unwrap_or(false) {
            return 1;
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn on_key(message: u32, event: &KBDLLHOOKSTRUCT) -> bool {
    if is_ours(event.dwExtraInfo) {
        return false;
    }
    let Ok(vk) = u16::try_from(event.vkCode) else {
        return false;
    };
    if vk == VK_PACKET || (vk == VK_LCONTROL && event.scanCode == ALTGR_CONTROL_SCAN) {
        return false;
    }
    let down = match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return false,
    };
    let Some(config) = current_config() else {
        return false;
    };
    let decision = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let HookState {
            machine,
            foreground,
        } = &mut *state;
        machine.resync(held_modifier_bits());
        if down {
            foreground.refresh();
        }
        let lookup = down && keys::modifier_bit(vk).is_none();
        let app = if lookup {
            foreground.app(&config.excluded_apps)
        } else {
            String::new()
        };
        let typed = (lookup && !config.char_swap_rules.is_empty())
            .then(|| {
                typed_text(
                    vk,
                    event.scanCode,
                    machine.physical_bits(),
                    foreground.layout(),
                )
            })
            .flatten();
        machine.key(
            vk,
            down,
            &Event {
                config: &config,
                app: &app,
                typed: typed.as_deref(),
                injectable: foreground.injectable(),
            },
        )
    });
    apply(decision)
}

fn on_mouse(message: u32, event: &MSLLHOOKSTRUCT) -> bool {
    let gesture = match message {
        WM_LBUTTONDOWN => Gesture::Button(MouseButton::Left, true),
        WM_LBUTTONUP => Gesture::Button(MouseButton::Left, false),
        WM_RBUTTONDOWN => Gesture::Button(MouseButton::Right, true),
        WM_RBUTTONUP => Gesture::Button(MouseButton::Right, false),
        WM_MOUSEWHEEL => Gesture::Wheel(false, wheel_delta(event.mouseData)),
        WM_MOUSEHWHEEL => Gesture::Wheel(true, wheel_delta(event.mouseData)),
        _ => return false,
    };
    if is_ours(event.dwExtraInfo) {
        return false;
    }
    let Some(config) = current_config() else {
        return false;
    };
    let decision = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let HookState {
            machine,
            foreground,
        } = &mut *state;
        machine.resync(held_modifier_bits());
        let lookup = !matches!(gesture, Gesture::Button(_, false));
        if lookup {
            foreground.refresh();
        }
        let app = if lookup {
            foreground.app(&config.excluded_apps)
        } else {
            String::new()
        };
        let event = Event {
            config: &config,
            app: &app,
            typed: None,
            injectable: foreground.injectable(),
        };
        match gesture {
            Gesture::Button(button, down) => machine.button(button, down, &event),
            Gesture::Wheel(horizontal, delta) => machine.wheel(horizontal, delta, &event),
        }
    });
    apply(decision)
}

fn apply(decision: Decision) -> bool {
    send(&decision.outputs);
    decision.swallow
}

fn current_config() -> Option<Arc<ResolvedConfig>> {
    let lock = CONFIG.get()?;
    Some(
        lock.read()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner())),
    )
}

fn is_ours(extra: usize) -> bool {
    keyremap_marker::decode(extra as i64).is_some()
}

fn tag() -> usize {
    keyremap_marker::encode(0, 0) as usize
}

fn wheel_delta(data: u32) -> i32 {
    i32::from((data >> 16) as u16 as i16)
}

fn held_modifier_bits() -> u8 {
    keys::MODIFIERS
        .iter()
        .filter(|(vk, _)| unsafe { GetAsyncKeyState(i32::from(*vk)) } < 0)
        .fold(0, |bits, (_, bit)| bits | bit)
}

fn typed_text(vk: u16, scan: u32, bits: u8, layout: HKL) -> Option<String> {
    let mods = modifiers_from_bits(bits);
    let mut state = [0u8; 256];
    if mods.shift {
        state[usize::from(VK_SHIFT)] = 0x80;
    }
    if mods.ralt {
        state[usize::from(VK_CONTROL)] = 0x80;
        state[usize::from(VK_MENU)] = 0x80;
    }
    if unsafe { GetKeyState(i32::from(VK_CAPITAL)) } & 1 != 0 {
        state[usize::from(VK_CAPITAL)] = 0x01;
    }
    let mut buffer = [0u16; 8];
    let written = unsafe {
        ToUnicodeEx(
            u32::from(vk),
            scan,
            state.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            NO_KEYBOARD_STATE_CHANGE,
            layout,
        )
    };
    let length = usize::try_from(written).ok().filter(|length| *length > 0)?;
    Some(String::from_utf16_lossy(
        &buffer[..length.min(buffer.len())],
    ))
}

fn send(outputs: &[Output]) {
    let inputs: Vec<INPUT> = outputs.iter().flat_map(inputs_for).collect();
    if inputs.is_empty() {
        return;
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        log::warn!(
            "SendInput delivered {sent} of {} events: {}",
            inputs.len(),
            std::io::Error::last_os_error()
        );
    }
}

fn inputs_for(output: &Output) -> Vec<INPUT> {
    match output {
        Output::Key { vk, down } => vec![key_input(*vk, *down)],
        Output::Mask => vec![
            key_input(keys::MASK_KEY, true),
            key_input(keys::MASK_KEY, false),
        ],
        Output::Text(text) => text
            .encode_utf16()
            .flat_map(|unit| [unicode_input(unit, true), unicode_input(unit, false)])
            .collect(),
        Output::Button { button, down } => {
            vec![mouse_input(button_flags(*button, *down), 0)]
        }
        Output::Wheel { horizontal, delta } => {
            let flags = if *horizontal {
                MOUSEEVENTF_HWHEEL
            } else {
                MOUSEEVENTF_WHEEL
            };
            vec![mouse_input(flags, *delta)]
        }
    }
}

fn key_input(vk: u16, down: bool) -> INPUT {
    let mut flags: KEYBD_EVENT_FLAGS = 0;
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    if keys::is_extended(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) };
    keyboard(KEYBDINPUT {
        wVk: vk,
        wScan: u16::try_from(scan).unwrap_or(0),
        dwFlags: flags,
        time: 0,
        dwExtraInfo: tag(),
    })
}

fn unicode_input(unit: u16, down: bool) -> INPUT {
    let flags = if down {
        KEYEVENTF_UNICODE
    } else {
        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
    };
    keyboard(KEYBDINPUT {
        wVk: 0,
        wScan: unit,
        dwFlags: flags,
        time: 0,
        dwExtraInfo: tag(),
    })
}

fn keyboard(ki: KEYBDINPUT) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki },
    }
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: tag(),
            },
        },
    }
}

fn button_flags(button: MouseButton, down: bool) -> MOUSE_EVENT_FLAGS {
    match (button, down) {
        (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
        (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_input_carries_the_keyremap_tag() {
        assert!(is_ours(tag()));
        for extra in [0usize, 1, 0x1234] {
            assert!(!is_ours(extra), "{extra:#x}");
        }
    }

    #[test]
    fn wheel_delta_reads_the_signed_high_word() {
        let cases = [
            (0x0078_0000u32, 120),
            (0xFF88_0000u32, -120),
            (0x0000_FFFFu32, 0),
            (0xFF10_0000u32, -240),
        ];
        for (data, expected) in cases {
            assert_eq!(wheel_delta(data), expected, "{data:#x}");
        }
    }

    #[test]
    fn button_flags_cover_both_buttons_and_directions() {
        let cases = [
            (MouseButton::Left, true, MOUSEEVENTF_LEFTDOWN),
            (MouseButton::Left, false, MOUSEEVENTF_LEFTUP),
            (MouseButton::Right, true, MOUSEEVENTF_RIGHTDOWN),
            (MouseButton::Right, false, MOUSEEVENTF_RIGHTUP),
        ];
        for (button, down, expected) in cases {
            assert_eq!(button_flags(button, down), expected, "{button:?} {down}");
        }
    }

    #[test]
    fn text_becomes_a_unicode_press_and_release_per_utf16_unit() {
        let cases = [("€", 2), ("ab", 4), ("😀", 4), ("", 0)];
        for (text, expected) in cases {
            assert_eq!(
                inputs_for(&Output::Text(text.to_string())).len(),
                expected,
                "{text}"
            );
        }
    }
}
