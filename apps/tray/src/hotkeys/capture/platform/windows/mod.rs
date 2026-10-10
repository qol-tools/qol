use super::super::binding::{Binding, CaptureEvent};
use super::super::{OnFire, RebuildBindings};
use super::hook_rules::{self, KeyEvent, KeyHistory};
use super::key_matcher::{self, KeyCombo, KeyMatcher};
use anyhow::{bail, Result};
use crossbeam_channel::Receiver;
use qol_hotkeys::grammar::{Key, Modifier as Mod};
use qol_hotkeys::windows_keycode;
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardLayout, GetLastInputInfo, SendInput, VkKeyScanExW, HKL, INPUT,
    INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, LASTINPUTINFO,
    VIRTUAL_KEY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
};

pub(crate) const KEEP_REGISTERED_ONE_SHOTS: bool = true;

const MENU_MASK_KEY: VIRTUAL_KEY = 0xE8;
const LIVENESS_INTERVAL_MS: u32 = 10_000;
const ARM_PENDING: u8 = 0;
const ARM_DONE: u8 = 1;
const ARM_CANCELLED: u8 = 2;

struct HookState {
    matcher: Arc<RwLock<KeyMatcher>>,
    fire_tx: Sender<CaptureEvent>,
}

static HOOK_STATE: OnceLock<HookState> = OnceLock::new();
static LAST_HOOK_TICK: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static HISTORY: RefCell<KeyHistory> = RefCell::new(KeyHistory::default());
    static LAYOUT: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn start_recording(_session_id: u64, _events: Arc<crate::daemon::EventBus>) -> bool {
    false
}

pub(crate) fn cancel_recording(_session_id: u64) {}

/// No capture backend re-emits events on Windows; nothing to flush.
pub(crate) fn release_held_keys() {}

pub(crate) fn install(
    bindings: Vec<Binding>,
    on_fire: OnFire,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) -> Result<()> {
    let matcher = Arc::new(RwLock::new(KeyMatcher::new(bindings, resolve_combo)));
    let fire_tx = key_matcher::spawn_fire_thread(on_fire)?;
    key_matcher::spawn_reload_thread(matcher.clone(), reload_rx, rebuild, fire_tx.clone());
    if HOOK_STATE.set(HookState { matcher, fire_tx }).is_err() {
        bail!("the Windows keyboard hook is already installed");
    }

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let arm_state = Arc::new(AtomicU8::new(ARM_PENDING));
    let hook_arm_state = arm_state.clone();
    std::thread::Builder::new()
        .name("hotkey-capture-windows".into())
        .spawn(move || run_hook(ready_tx, hook_arm_state))?;

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => bail!(message),
        Err(RecvTimeoutError::Timeout) => {
            match arm_state.compare_exchange(
                ARM_PENDING,
                ARM_CANCELLED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => bail!("Windows keyboard hook did not report readiness"),
                Err(_) => Ok(()),
            }
        }
        Err(RecvTimeoutError::Disconnected) => {
            bail!("Windows keyboard hook exited before reporting readiness")
        }
    }
}

fn run_hook(ready_tx: Sender<Result<(), String>>, arm_state: Arc<AtomicU8>) {
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
    let mut hook = match arm() {
        Ok(hook) => hook,
        Err(message) => {
            let _ = ready_tx.send(Err(message));
            return;
        }
    };
    if arm_state
        .compare_exchange(ARM_PENDING, ARM_DONE, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        unsafe { UnhookWindowsHookEx(hook) };
        qol_runtime::probe!("HOTKEY_HOOK", "phase=arm outcome=cancelled-after-timeout");
        return;
    }
    if let Some(state) = HOOK_STATE.get() {
        log::info!(
            "Windows keyboard hook armed with {} bindings",
            state.matcher.read().map(|m| m.binding_count()).unwrap_or(0)
        );
    }
    let _ = ready_tx.send(Ok(()));
    if let Some(tick) = last_input_tick() {
        LAST_HOOK_TICK.store(tick, Ordering::Relaxed);
    }
    unsafe { SetTimer(std::ptr::null_mut(), 0, LIVENESS_INTERVAL_MS, None) };
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        if message.message == WM_TIMER {
            hook = rearm_if_stale(hook);
        }
    }
}

fn arm() -> Result<HHOOK, String> {
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
    if hook.is_null() {
        return Err(format!(
            "failed to install the Windows keyboard hook: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(hook)
}

fn rearm_if_stale(hook: HHOOK) -> HHOOK {
    let Some(input_tick) = last_input_tick() else {
        return hook;
    };
    let hook_tick = LAST_HOOK_TICK.load(Ordering::Relaxed);
    if !hook_rules::needs_rearm(input_tick, hook_tick) {
        return hook;
    }
    match arm() {
        Ok(fresh) => {
            unsafe { UnhookWindowsHookEx(hook) };
            LAST_HOOK_TICK.store(input_tick, Ordering::Relaxed);
            qol_runtime::probe!(
                "HOTKEY_HOOK",
                "phase=rearm unseen_input_ms={}",
                input_tick.wrapping_sub(hook_tick)
            );
            fresh
        }
        Err(message) => {
            log::warn!("Windows keyboard hook re-arm failed: {message}");
            hook
        }
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
    if code == HC_ACTION as i32
        && handle_event(
            unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) },
            wparam as u32,
        )
    {
        return 1;
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn handle_event(event: &KBDLLHOOKSTRUCT, message: u32) -> bool {
    LAST_HOOK_TICK.store(event.time, Ordering::Relaxed);
    if event.flags & LLKHF_INJECTED != 0 {
        return false;
    }
    let Some(state) = HOOK_STATE.get() else {
        return false;
    };
    handle_key(state, message, event)
}

fn handle_key(state: &HookState, message: u32, event: &KBDLLHOOKSTRUCT) -> bool {
    let down = match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => true,
        WM_KEYUP | WM_SYSKEYUP => false,
        _ => return false,
    };
    let key = event.vkCode as VIRTUAL_KEY;
    let key_event = KeyEvent {
        key,
        scan: event.scanCode,
        down,
        time: event.time,
    };
    if down {
        refresh_layout(&state.matcher);
    }
    let swallowed = is_swallowing(&state.matcher, key);
    let async_down = key_is_down(key);
    let observed_transition = HISTORY.with(|history| {
        let mut history = history.borrow_mut();
        if history.note_altgr(key_event) {
            return None;
        }
        Some((
            history.observe(key_event, swallowed, async_down),
            history.synthetic_ctrl(),
        ))
    });
    let Some((transition, synthetic_ctrl)) = observed_transition else {
        return false;
    };
    let observed = KeyCombo {
        mods: hook_rules::held_mods(key_is_down, synthetic_ctrl),
        key,
    };
    let outcome = key_matcher::match_under_lock(&state.matcher, transition, &observed);
    if hook_rules::needs_menu_mask(transition, outcome.swallow, &observed.mods) {
        send_menu_mask();
    }
    if let Some((binding, phase)) = outcome.fired {
        let _ = state.fire_tx.send(CaptureEvent { binding, phase });
    }
    outcome.swallow
}

fn refresh_layout(matcher: &RwLock<KeyMatcher>) {
    let layout = foreground_layout() as usize;
    if LAYOUT.with(|current| current.replace(layout)) == layout {
        return;
    }
    match matcher.write() {
        Ok(mut guard) => guard.re_resolve(),
        Err(poisoned) => poisoned.into_inner().re_resolve(),
    }
    qol_runtime::probe!("HOTKEY_LAYOUT", "layout={:#x}", layout);
}

fn foreground_layout() -> HKL {
    let window = unsafe { GetForegroundWindow() };
    let thread = unsafe { GetWindowThreadProcessId(window, std::ptr::null_mut()) };
    unsafe { GetKeyboardLayout(thread) }
}

fn key_is_down(key: u16) -> bool {
    (unsafe { GetAsyncKeyState(i32::from(key)) }) < 0
}

fn is_swallowing(matcher: &RwLock<KeyMatcher>, key: VIRTUAL_KEY) -> bool {
    match matcher.read() {
        Ok(guard) => guard.is_swallowing(key),
        Err(poisoned) => poisoned.into_inner().is_swallowing(key),
    }
}

fn send_menu_mask() {
    let inputs = [
        keyboard_input(MENU_MASK_KEY, 0),
        keyboard_input(MENU_MASK_KEY, KEYEVENTF_KEYUP),
    ];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
}

fn keyboard_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn resolve_combo(binding: &Binding) -> Option<KeyCombo> {
    let combo = binding.combo.as_ref()?;
    let mut mods = combo.mods.clone();
    let key = match combo.key {
        Key::Symbol(symbol) => {
            let (key, symbol_mods) = symbol_key(symbol)?;
            mods.extend(symbol_mods);
            key
        }
        key => windows_keycode::key_to_vk(key)?,
    };
    Some(KeyCombo { mods, key })
}

fn symbol_key(symbol: char) -> Option<(VIRTUAL_KEY, BTreeSet<Mod>)> {
    let unit = u16::try_from(u32::from(symbol)).ok()?;
    hook_rules::decode_vk_scan(unsafe { VkKeyScanExW(unit, foreground_layout()) })
}
