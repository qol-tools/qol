use std::sync::{Arc, OnceLock, RwLock};

use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use foreign_types_shared::ForeignType;

extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

use super::app_tracker::AppTracker;
use crate::remap::{
    self, KeyAction, Modifiers, MouseAction, MouseButton, ResolvedConfig, ScrollAction,
};

pub struct TapState {
    config: RwLock<Arc<ResolvedConfig>>,
    app_tracker: Arc<AppTracker>,
}

impl TapState {
    pub fn new(config: ResolvedConfig, app_tracker: Arc<AppTracker>) -> Self {
        Self {
            config: RwLock::new(Arc::new(config)),
            app_tracker,
        }
    }

    pub fn swap_config(&self, new_config: ResolvedConfig) {
        let new = Arc::new(new_config);
        if let Ok(mut guard) = self.config.write() {
            *guard = new;
        }
    }

    fn config(&self) -> Arc<ResolvedConfig> {
        self.config
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }
}

pub fn start_tap(state: Arc<TapState>) {
    std::thread::Builder::new()
        .name("keyremap-tap".into())
        .spawn(move || run_tap(state))
        .expect("failed to spawn tap thread");
}

fn wait_for_accessibility() {
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    if unsafe { AXIsProcessTrusted() } {
        return;
    }

    eprintln!("[keyremap] waiting for Accessibility permission...");
    eprintln!("[keyremap] grant in System Settings > Privacy & Security > Accessibility");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if unsafe { AXIsProcessTrusted() } {
            eprintln!("[keyremap] Accessibility permission granted");
            return;
        }
    }
}

fn run_tap(state: Arc<TapState>) {
    wait_for_accessibility();

    let events = vec![
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::ScrollWheel,
    ];

    // The callback cannot borrow the tap (the tap owns the callback), so the
    // tap's mach port is published here and read back in the callback to re-arm
    // the tap when macOS disables it. Without re-arming, the tap stays dead
    // after the first disable and all remapping silently stops until restart.
    let tap_port: Arc<OnceLock<usize>> = Arc::new(OnceLock::new());
    let tap_port_cb = Arc::clone(&tap_port);

    let tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        events,
        move |_proxy, event_type, event| {
            if matches!(
                event_type,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                if let Some(&port) = tap_port_cb.get() {
                    unsafe { CGEventTapEnable(port as CFMachPortRef, true) };
                }
                return CallbackResult::Keep;
            }
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_event(&state, event_type, event)
            })) {
                Ok(result) => result,
                Err(_) => {
                    eprintln!("[keyremap] panic in event callback — passing event through");
                    CallbackResult::Keep
                }
            }
        },
    );

    let tap = match tap {
        Ok(tap) => tap,
        Err(()) => {
            eprintln!("[keyremap] failed to create event tap (even with Accessibility granted)");
            std::process::exit(1);
        }
    };

    let _ = tap_port.set(tap.mach_port().as_concrete_TypeRef() as usize);

    let loop_source = tap
        .mach_port()
        .create_runloop_source(0)
        .expect("failed to create run loop source for event tap");
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    CFRunLoop::run_current();
}

fn handle_event(
    state: &TapState,
    event_type: CGEventType,
    event: &core_graphics::event::CGEvent,
) -> CallbackResult {
    if matches!(event_type, CGEventType::FlagsChanged) {
        return CallbackResult::Keep;
    }

    let config = state.config();
    if !config.enabled {
        return CallbackResult::Keep;
    }
    let bundle_id = state.app_tracker.bundle_id();

    match event_type {
        CGEventType::KeyDown | CGEventType::KeyUp => {
            handle_key_event(config.as_ref(), event, &bundle_id)
        }
        CGEventType::LeftMouseDown | CGEventType::LeftMouseUp => {
            handle_mouse_event(config.as_ref(), event, MouseButton::Left, &bundle_id)
        }
        CGEventType::RightMouseDown | CGEventType::RightMouseUp => {
            handle_mouse_event(config.as_ref(), event, MouseButton::Right, &bundle_id)
        }
        CGEventType::ScrollWheel => handle_scroll_event(config.as_ref(), event, &bundle_id),
        _ => CallbackResult::Keep,
    }
}

fn event_character(event: &core_graphics::event::CGEvent) -> Option<String> {
    extern "C" {
        fn CGEventKeyboardGetUnicodeString(
            event: core_graphics::sys::CGEventRef,
            max_len: core::ffi::c_ulong,
            actual_len: *mut core::ffi::c_ulong,
            buf: *mut u16,
        );
    }
    let mut buf = [0u16; 4];
    let mut len: core::ffi::c_ulong = 0;
    unsafe {
        CGEventKeyboardGetUnicodeString(
            event.as_ptr(),
            buf.len() as core::ffi::c_ulong,
            &mut len,
            buf.as_mut_ptr(),
        );
    }
    if len == 0 {
        return None;
    }
    String::from_utf16(&buf[..len as usize]).ok()
}

fn handle_key_event(
    config: &ResolvedConfig,
    event: &core_graphics::event::CGEvent,
    bundle_id: &str,
) -> CallbackResult {
    let flags = event.get_flags();
    let mods = extract_modifiers(flags);
    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
    let event_char = if config.char_swap_rules.is_empty() {
        None
    } else {
        event_character(event)
    };

    let action = remap::process_key_event(config, mods, keycode, event_char.as_deref(), bundle_id);

    #[cfg(debug_assertions)]
    if !matches!(action, KeyAction::Passthrough) || config.excluded_apps.contains(bundle_id) {
        eprintln!(
            "[keyremap:dbg] app={} key=0x{:02X}({}) mods={:?} -> {:?}",
            bundle_id,
            keycode,
            crate::keycode::key_name(keycode),
            mods,
            action,
        );
    }

    match action {
        KeyAction::Passthrough => CallbackResult::Keep,
        KeyAction::Remap {
            mods: new_mods,
            key,
        } => {
            let new_flags = build_flags(flags, mods, new_mods);
            event.set_flags(new_flags);
            event.set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, key as i64);
            CallbackResult::Keep
        }
        KeyAction::Char { ref text } => {
            let clean_flags = strip_all_modifiers(flags);
            event.set_flags(clean_flags);
            // Set keycode to SPACE so dead-key positions (like ´ on Nordic)
            // don't trigger the input method's dead-key state machine.
            event.set_integer_value_field(
                EventField::KEYBOARD_EVENT_KEYCODE,
                crate::keycode::SPACE as i64,
            );
            event.set_string(text);
            CallbackResult::Keep
        }
    }
}

fn handle_mouse_event(
    config: &ResolvedConfig,
    event: &core_graphics::event::CGEvent,
    button: MouseButton,
    bundle_id: &str,
) -> CallbackResult {
    let flags = event.get_flags();
    let mods = extract_modifiers(flags);

    match remap::process_mouse_event(config, mods, button, bundle_id) {
        MouseAction::Passthrough => CallbackResult::Keep,
        MouseAction::Remap { mods: new_mods } => {
            let new_flags = build_flags(flags, mods, new_mods);
            event.set_flags(new_flags);
            CallbackResult::Keep
        }
    }
}

fn handle_scroll_event(
    config: &ResolvedConfig,
    event: &core_graphics::event::CGEvent,
    bundle_id: &str,
) -> CallbackResult {
    let flags = event.get_flags();
    let mods = extract_modifiers(flags);

    match remap::process_scroll_event(config, mods, bundle_id) {
        ScrollAction::Passthrough => CallbackResult::Keep,
        ScrollAction::Remap { mods: new_mods } => {
            let new_flags = build_flags(flags, mods, new_mods);
            event.set_flags(new_flags);
            CallbackResult::Keep
        }
    }
}

fn strip_all_modifiers(flags: CGEventFlags) -> CGEventFlags {
    let mut f = flags;
    f.remove(CGEventFlags::CGEventFlagControl);
    f.remove(CGEventFlags::CGEventFlagShift);
    f.remove(CGEventFlags::CGEventFlagAlternate);
    f.remove(CGEventFlags::CGEventFlagCommand);
    f
}

/// NX_DEVICERALTKEYMASK — device-dependent bit for Right Alt/Option.
const NX_DEVICERALTKEYMASK: u64 = 0x40;

fn extract_modifiers(flags: CGEventFlags) -> Modifiers {
    Modifiers {
        ctrl: flags.contains(CGEventFlags::CGEventFlagControl),
        shift: flags.contains(CGEventFlags::CGEventFlagShift),
        alt: flags.contains(CGEventFlags::CGEventFlagAlternate),
        cmd: flags.contains(CGEventFlags::CGEventFlagCommand),
        ralt: (flags.bits() & NX_DEVICERALTKEYMASK) != 0,
    }
}

fn build_flags(original: CGEventFlags, from: Modifiers, to: Modifiers) -> CGEventFlags {
    let mut flags = original;

    if from.ctrl && !to.ctrl {
        flags.remove(CGEventFlags::CGEventFlagControl);
    }
    if from.shift && !to.shift {
        flags.remove(CGEventFlags::CGEventFlagShift);
    }
    if from.alt && !to.alt {
        flags.remove(CGEventFlags::CGEventFlagAlternate);
    }
    if from.cmd && !to.cmd {
        flags.remove(CGEventFlags::CGEventFlagCommand);
    }

    if !from.ctrl && to.ctrl {
        flags.insert(CGEventFlags::CGEventFlagControl);
    }
    if !from.shift && to.shift {
        flags.insert(CGEventFlags::CGEventFlagShift);
    }
    if !from.alt && to.alt {
        flags.insert(CGEventFlags::CGEventFlagAlternate);
    }
    if !from.cmd && to.cmd {
        flags.insert(CGEventFlags::CGEventFlagCommand);
    }

    flags
}
