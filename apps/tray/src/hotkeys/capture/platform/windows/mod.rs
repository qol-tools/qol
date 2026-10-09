use super::super::binding::{Binding, CaptureEvent};
use super::super::{OnFire, RebuildBindings};
use super::key_matcher::{self, KeyCombo, KeyMatcher, KeyTransition};
use anyhow::{bail, Result};
use crossbeam_channel::Receiver;
use qol_hotkeys::grammar::{Key, Modifier as Mod};
use qol_hotkeys::windows_keycode;
use std::collections::BTreeSet;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
    VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

const MENU_MASK_KEY: VIRTUAL_KEY = 0xE8;

struct HookState {
    matcher: Arc<RwLock<KeyMatcher>>,
    fire_tx: Sender<CaptureEvent>,
}

static HOOK_STATE: OnceLock<HookState> = OnceLock::new();

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
    std::thread::Builder::new()
        .name("hotkey-capture-windows".into())
        .spawn(move || run_hook(ready_tx))?;

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => bail!(message),
        Err(RecvTimeoutError::Timeout) => bail!("Windows keyboard hook did not report readiness"),
        Err(RecvTimeoutError::Disconnected) => {
            bail!("Windows keyboard hook exited before reporting readiness")
        }
    }
}

fn run_hook(ready_tx: Sender<Result<(), String>>) {
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
    if hook.is_null() {
        let _ = ready_tx.send(Err(format!(
            "failed to install the Windows keyboard hook: {}",
            std::io::Error::last_os_error()
        )));
        return;
    }
    if let Some(state) = HOOK_STATE.get() {
        log::info!(
            "Windows keyboard hook armed with {} bindings",
            state.matcher.read().map(|m| m.binding_count()).unwrap_or(0)
        );
    }
    let _ = ready_tx.send(Ok(()));
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {}
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(state) = HOOK_STATE.get() {
            let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if handle_key(state, wparam as u32, event.vkCode as VIRTUAL_KEY) {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

fn handle_key(state: &HookState, message: u32, key: VIRTUAL_KEY) -> bool {
    if key == MENU_MASK_KEY {
        return false;
    }
    let transition = match message {
        WM_KEYUP | WM_SYSKEYUP => KeyTransition::Release,
        WM_KEYDOWN | WM_SYSKEYDOWN if is_swallowing(&state.matcher, key) => KeyTransition::Repeat,
        WM_KEYDOWN | WM_SYSKEYDOWN => KeyTransition::Press,
        _ => return false,
    };
    let observed = KeyCombo {
        mods: held_mods(),
        key,
    };
    let outcome = key_matcher::match_under_lock(&state.matcher, transition, &observed);
    if transition == KeyTransition::Press
        && outcome.swallow
        && (observed.mods.contains(&Mod::Super) || observed.mods.contains(&Mod::Alt))
    {
        send_menu_mask();
    }
    if let Some((binding, phase)) = outcome.fired {
        let _ = state.fire_tx.send(CaptureEvent { binding, phase });
    }
    outcome.swallow
}

fn is_swallowing(matcher: &RwLock<KeyMatcher>, key: VIRTUAL_KEY) -> bool {
    match matcher.read() {
        Ok(guard) => guard.is_swallowing(key),
        Err(poisoned) => poisoned.into_inner().is_swallowing(key),
    }
}

fn held_mods() -> BTreeSet<Mod> {
    [
        (VK_SHIFT, Mod::Shift),
        (VK_CONTROL, Mod::Ctrl),
        (VK_MENU, Mod::Alt),
        (VK_LWIN, Mod::Super),
        (VK_RWIN, Mod::Super),
    ]
    .into_iter()
    .filter(|(key, _)| unsafe { GetAsyncKeyState(i32::from(*key)) } < 0)
    .map(|(_, modifier)| modifier)
    .collect()
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
    let key = match combo.key {
        Key::Symbol(symbol) => symbol_key(symbol)?,
        key => windows_keycode::key_to_vk(key)?,
    };
    Some(KeyCombo {
        mods: combo.mods.clone(),
        key,
    })
}

fn symbol_key(symbol: char) -> Option<VIRTUAL_KEY> {
    let unit = u16::try_from(u32::from(symbol)).ok()?;
    let scan = unsafe { VkKeyScanW(unit) };
    (scan != -1).then_some(scan as VIRTUAL_KEY & 0xFF)
}
