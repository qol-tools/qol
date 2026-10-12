use std::cell::RefCell;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use qol_runtime::hook_liveness::SeenInput;
use qol_runtime::keyremap_marker;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetLastInputInfo, MapVirtualKeyW, SendInput, ToUnicodeEx, HKL,
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, LASTINPUTINFO, MAPVK_VK_TO_VSC,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, MOUSE_EVENT_FLAGS, VK_CAPITAL, VK_CONTROL,
    VK_LCONTROL, VK_MENU, VK_PACKET, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW,
    UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER,
};

use super::elevation;
use super::foreground::Foreground;
use super::keys;
use super::machine::{Decision, Event, Machine, Output};
use super::status::{self, HookStatus};
use crate::platform::engine::daemon;
use crate::platform::engine::remap::{modifiers_from_bits, MouseButton, ResolvedConfig};
use crate::platform::engine::Remapper;

const ALTGR_CONTROL_SCAN: u32 = 0x21D;
const NO_KEYBOARD_STATE_CHANGE: u32 = 0x4;
const READY_TIMEOUT: Duration = Duration::from_secs(2);
const GAP_CHECK_MS: u32 = 1_000;
const REARM_INTERVAL: Duration = Duration::from_secs(10);

static CONFIG: OnceLock<RwLock<Arc<ResolvedConfig>>> = OnceLock::new();
static SEEN_INPUT: SeenInput = SeenInput::new();

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Liveness {
    Seen,
    Gap,
    Rearm,
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
                watch_stop_request(thread_id);
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
        if !end_hook_thread(self.thread_id) {
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

struct Installed {
    keyboard: HHOOK,
    mouse: HHOOK,
}

impl Installed {
    fn arm() -> Result<Self, String> {
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
        if keyboard.is_null() {
            return Err(format!(
                "failed to install the Windows keyboard hook: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
        if mouse.is_null() {
            let error = format!(
                "failed to install the Windows mouse hook: {}",
                std::io::Error::last_os_error()
            );
            unsafe { UnhookWindowsHookEx(keyboard) };
            return Err(error);
        }
        Ok(Self { keyboard, mouse })
    }

    fn unhook(self) {
        unsafe {
            UnhookWindowsHookEx(self.mouse);
            UnhookWindowsHookEx(self.keyboard);
        }
    }
}

fn watch_stop_request(thread_id: u32) {
    let watcher = std::thread::Builder::new()
        .name("keyremap-stop".into())
        .spawn(move || {
            end_on_stop(qol_process::wait_for_stop_request(), || {
                daemon::send_kill() || end_hook_thread(thread_id)
            })
        });
    if let Err(error) = watcher {
        log::warn!("the Key Remap stop request listener did not start: {error}");
    }
}

fn end_on_stop(request: std::io::Result<()>, end: impl FnOnce() -> bool) -> bool {
    match request {
        Ok(()) => {
            let ended = end();
            if !ended {
                log::warn!(
                    "could not end the Windows input hooks on a stop request: {}",
                    std::io::Error::last_os_error()
                );
            }
            ended
        }
        Err(error) => {
            log::warn!("the Key Remap stop request listener is unavailable: {error}");
            false
        }
    }
}

fn end_hook_thread(thread_id: u32) -> bool {
    unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) != 0 }
}

fn run(ready: Sender<Result<u32, String>>) {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    let mut hooks = match Installed::arm() {
        Ok(hooks) => hooks,
        Err(error) => {
            status::write(&HookStatus::failed(std::process::id(), error.clone()));
            let _ = ready.send(Err(error));
            return;
        }
    };
    STATE.with(|state| state.borrow_mut().foreground = Foreground::new(elevation::current()));
    let mut hook_status = HookStatus::installed(std::process::id());
    status::write(&hook_status);
    if let Some(tick) = last_input_tick() {
        SEEN_INPUT.rearmed(tick);
    }
    let mut last_arm = Instant::now();
    unsafe { SetTimer(std::ptr::null_mut(), 0, GAP_CHECK_MS, None) };
    let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        if message.message == WM_TIMER && check_liveness(&mut hooks, last_arm.elapsed()) {
            last_arm = Instant::now();
            hook_status.rearmed(status::unix_now());
            status::write(&hook_status);
        }
    }
    hooks.unhook();
    let outputs = STATE.with(|state| state.borrow_mut().machine.release_all());
    send(&outputs, tag());
    status::write(&hook_status.stopped());
}

fn check_liveness(hooks: &mut Installed, since_arm: Duration) -> bool {
    let Some(input_tick) = last_input_tick() else {
        return false;
    };
    let injectable = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.foreground.refresh();
        state.foreground.injectable()
    });
    let verdict = liveness(SEEN_INPUT.unseen(input_tick), injectable, since_arm);
    if verdict == Liveness::Seen {
        return false;
    }
    let outputs = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.machine.gap();
        if injectable {
            state.machine.take_gap()
        } else {
            Vec::new()
        }
    });
    send(&outputs, tag());
    if verdict != Liveness::Rearm {
        return false;
    }
    match Installed::arm() {
        Ok(fresh) => {
            std::mem::replace(hooks, fresh).unhook();
            SEEN_INPUT.rearmed(input_tick);
            log::warn!("re-armed the Windows input hooks after input they did not see");
            true
        }
        Err(error) => {
            log::warn!("could not re-arm the Windows input hooks: {error}");
            false
        }
    }
}

fn liveness(unseen: bool, injectable: bool, since_arm: Duration) -> Liveness {
    if !unseen {
        Liveness::Seen
    } else if injectable && since_arm >= REARM_INTERVAL {
        Liveness::Rearm
    } else {
        Liveness::Gap
    }
}

fn last_input_tick() -> Option<u32> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    (unsafe { GetLastInputInfo(&mut info) } != 0).then_some(info.dwTime)
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        SEEN_INPUT.keyboard(event.time);
        if std::panic::catch_unwind(|| on_key(wparam as u32, event)).unwrap_or(false) {
            return 1;
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
        SEEN_INPUT.mouse(event.time);
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
    let (decision, marker) = STATE.with(|state| {
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
        let marker = marker_for(machine.physical_bits(), vk);
        let decision = machine.key(
            vk,
            down,
            &Event {
                config: &config,
                app: &app,
                typed: typed.as_deref(),
                injectable: foreground.injectable(),
            },
        );
        (decision, marker)
    });
    apply(decision, marker)
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
    apply(decision, tag())
}

fn apply(decision: Decision, marker: usize) -> bool {
    send(&decision.outputs, marker);
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

fn marker_for(bits: u8, vk: u16) -> usize {
    let mods = modifiers_from_bits(bits);
    let marker_mods = [
        (mods.ctrl, keyremap_marker::MOD_CTRL),
        (mods.shift, keyremap_marker::MOD_SHIFT),
        (mods.alt, keyremap_marker::MOD_ALT),
        (mods.cmd, keyremap_marker::MOD_SUPER),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(0, |marker, (_, bit)| marker | bit);
    keyremap_marker::encode(marker_mods, vk) as usize
}

fn send(outputs: &[Output], marker: usize) {
    let inputs: Vec<INPUT> = outputs
        .iter()
        .flat_map(|output| inputs_for(output, marker))
        .collect();
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

fn inputs_for(output: &Output, marker: usize) -> Vec<INPUT> {
    match output {
        Output::Key { vk, down } => vec![key_input(*vk, *down, key_extra(*vk, marker))],
        Output::Mask => vec![
            key_input(keys::MASK_KEY, true, tag()),
            key_input(keys::MASK_KEY, false, tag()),
        ],
        Output::Text(text) => text_inputs(text, marker),
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

fn key_extra(vk: u16, marker: usize) -> usize {
    if keys::modifier_bit(vk).is_some() {
        tag()
    } else {
        marker
    }
}

fn text_inputs(text: &str, marker: usize) -> Vec<INPUT> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let last = units.len().saturating_sub(1);
    units
        .iter()
        .enumerate()
        .flat_map(|(index, unit)| {
            let press = if index == 0 { marker } else { tag() };
            let release = if index == last { marker } else { tag() };
            [
                unicode_input(*unit, true, press),
                unicode_input(*unit, false, release),
            ]
        })
        .collect()
}

fn key_input(vk: u16, down: bool, extra: usize) -> INPUT {
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
        dwExtraInfo: extra,
    })
}

fn unicode_input(unit: u16, down: bool, extra: usize) -> INPUT {
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
        dwExtraInfo: extra,
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
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_B, VK_SPACE};

    fn key(vk: u16, down: bool) -> Output {
        Output::Key { vk, down }
    }

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
                inputs_for(&Output::Text(text.to_string()), tag()).len(),
                expected,
                "{text}"
            );
        }
    }

    #[test]
    fn only_unseen_input_acts_and_only_a_reachable_foreground_rearms() {
        let long = REARM_INTERVAL;
        let short = REARM_INTERVAL / 2;
        let cases = [
            (false, true, long, Liveness::Seen),
            (false, false, long, Liveness::Seen),
            (true, true, long, Liveness::Rearm),
            (true, true, short, Liveness::Gap),
            (true, false, long, Liveness::Gap),
        ];
        for (unseen, injectable, since_arm, expected) in cases {
            assert_eq!(
                liveness(unseen, injectable, since_arm),
                expected,
                "unseen={unseen} injectable={injectable} since_arm={since_arm:?}"
            );
        }
    }

    #[test]
    fn a_stop_request_ends_the_hook_thread_only_when_it_arrives() {
        let mut ended = false;
        assert!(end_on_stop(Ok(()), || {
            ended = true;
            true
        }));
        assert!(ended);
        let mut ended = false;
        assert!(!end_on_stop(
            Err(std::io::Error::other("no stop event")),
            || {
                ended = true;
                true
            }
        ));
        assert!(!ended);
        assert!(!end_on_stop(Ok(()), || false));
    }

    #[test]
    fn the_marker_names_the_physical_chord() {
        let cases = [
            (0x00, VK_A, 0),
            (0x01, VK_A, keyremap_marker::MOD_CTRL),
            (0x10, VK_A, keyremap_marker::MOD_CTRL),
            (0x22, VK_A, keyremap_marker::MOD_SHIFT),
            (0x40, VK_A, keyremap_marker::MOD_ALT),
            (0x08, VK_SPACE, keyremap_marker::MOD_SUPER),
            (
                0x0F,
                VK_SPACE,
                keyremap_marker::MOD_CTRL
                    | keyremap_marker::MOD_SHIFT
                    | keyremap_marker::MOD_ALT
                    | keyremap_marker::MOD_SUPER,
            ),
        ];
        for (bits, vk, mods) in cases {
            let marker = keyremap_marker::decode(marker_for(bits, vk) as i64).expect("marker");
            assert_eq!((marker.mods, marker.key), (mods, vk), "bits={bits:#x}");
        }
    }

    #[test]
    fn only_remapped_keys_and_text_carry_the_marker() {
        let marker = marker_for(0x01, VK_A);
        let extras = |output: Output| -> Vec<usize> {
            inputs_for(&output, marker)
                .iter()
                .map(|input| match input.r#type {
                    INPUT_KEYBOARD => unsafe { input.Anonymous.ki.dwExtraInfo },
                    _ => unsafe { input.Anonymous.mi.dwExtraInfo },
                })
                .collect()
        };
        let cases = [
            (key(VK_B, true), vec![marker]),
            (key(VK_B, false), vec![marker]),
            (key(VK_LCONTROL, true), vec![tag()]),
            (Output::Mask, vec![tag(), tag()]),
            (Output::Text("\u{20ac}".into()), vec![marker, marker]),
            (
                Output::Text("ab".into()),
                vec![marker, tag(), tag(), marker],
            ),
            (
                Output::Button {
                    button: MouseButton::Left,
                    down: true,
                },
                vec![tag()],
            ),
            (
                Output::Wheel {
                    horizontal: false,
                    delta: 120,
                },
                vec![tag()],
            ),
        ];
        for (output, expected) in cases {
            let name = format!("{output:?}");
            assert_eq!(extras(output), expected, "{name}");
        }
    }
}
