use std::mem::ManuallyDrop;
use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use core_foundation::base::TCFType;
use core_foundation::mach_port::{CFMachPort, CFMachPortInvalidate, CFMachPortRef};
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventFlags, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy,
    CGEventType, CallbackResult, EventField,
};
use core_graphics::sys::CGEventRef;
use foreign_types_shared::ForeignType;
use qol_hotkeys::macos_keycode as keycode;
use qol_runtime::keyremap_marker;

type RawEventTapCallback = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events_of_interest: u64,
        callback: RawEventTapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;

    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

use super::app::remap::{
    self, KeyAction, Modifiers, MouseAction, MouseButton, ResolvedConfig, ScrollAction,
};
use super::app_tracker::AppTracker;

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
    if accessibility_trusted() {
        return;
    }

    eprintln!("[keyremap] waiting for Accessibility permission...");
    eprintln!("[keyremap] grant in System Settings > Privacy & Security > Accessibility");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if accessibility_trusted() {
            eprintln!("[keyremap] Accessibility permission granted");
            return;
        }
    }
}

pub(super) fn accessibility_trusted() -> bool {
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    unsafe { AXIsProcessTrusted() }
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

    let tap = RawEventTap::new(
        CGEventTapLocation::AnnotatedSession,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        events,
        state,
    );

    let tap = match tap {
        Ok(tap) => tap,
        Err(()) => {
            eprintln!("[keyremap] failed to create event tap (even with Accessibility granted)");
            std::process::exit(1);
        }
    };

    let loop_source = tap
        .mach_port()
        .create_runloop_source(0)
        .expect("failed to create run loop source for event tap");
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    CFRunLoop::run_current();
}

struct RawEventTap {
    mach_port: CFMachPort,
    _callback_state: Box<RawEventTapState>,
}

struct RawEventTapState {
    state: Arc<TapState>,
    tap_port: AtomicUsize,
}

impl RawEventTap {
    fn new(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        events: Vec<CGEventType>,
        state: Arc<TapState>,
    ) -> Result<Self, ()> {
        let callback_state = Box::new(RawEventTapState {
            state,
            tap_port: AtomicUsize::new(0),
        });
        let callback_ptr = Box::into_raw(callback_state);
        let port = unsafe {
            CGEventTapCreate(
                tap,
                place,
                options,
                event_mask(&events),
                event_tap_callback,
                callback_ptr.cast(),
            )
        };

        if port.is_null() {
            let _ = unsafe { Box::from_raw(callback_ptr) };
            return Err(());
        }

        unsafe {
            (*callback_ptr)
                .tap_port
                .store(port as usize, Ordering::SeqCst);
        }
        Ok(Self {
            mach_port: unsafe { CFMachPort::wrap_under_create_rule(port) },
            _callback_state: unsafe { Box::from_raw(callback_ptr) },
        })
    }

    fn mach_port(&self) -> &CFMachPort {
        &self.mach_port
    }

    fn enable(&self) {
        unsafe { CGEventTapEnable(self.mach_port.as_concrete_TypeRef(), true) }
    }
}

impl Drop for RawEventTap {
    fn drop(&mut self) {
        unsafe { CFMachPortInvalidate(self.mach_port.as_concrete_TypeRef()) };
    }
}

fn event_mask(events: &[CGEventType]) -> u64 {
    events.iter().fold(0, |mask, event_type| {
        let bit = *event_type as u32;
        if bit < 64 {
            mask | (1u64 << bit)
        } else {
            mask
        }
    })
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event_ref: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        event_tap_callback_inner(event_type, event_ref, user_info)
    })) {
        Ok(event_ref) => event_ref,
        Err(_) => {
            eprintln!("[keyremap] panic in event callback - passing event through");
            event_ref
        }
    }
}

fn event_tap_callback_inner(
    event_type: CGEventType,
    event_ref: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    let Some(callback_state) = (unsafe { user_info.cast::<RawEventTapState>().as_ref() }) else {
        return event_ref;
    };

    if matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    ) {
        let port = callback_state.tap_port.load(Ordering::SeqCst);
        if port != 0 {
            unsafe { CGEventTapEnable(port as CFMachPortRef, true) };
        }
        return event_ref;
    }

    if event_ref.is_null() {
        return event_ref;
    }

    let event = unsafe { ManuallyDrop::new(core_graphics::event::CGEvent::from_ptr(event_ref)) };
    match handle_event(&callback_state.state, event_type, &event) {
        CallbackResult::Keep => event.as_ptr(),
        CallbackResult::Drop => ptr::null_mut(),
        CallbackResult::Replace(new_event) => ManuallyDrop::new(new_event).as_ptr(),
    }
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
    let target_pid =
        i32::try_from(event.get_integer_value_field(EventField::EVENT_TARGET_UNIX_PROCESS_ID))
            .unwrap_or_default();
    let bundle_id = state.app_tracker.bundle_id_for_target(target_pid);

    match event_type {
        CGEventType::KeyDown | CGEventType::KeyUp => {
            handle_key_event(config.as_ref(), event, target_pid, &bundle_id)
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
    target_pid: i32,
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
            "[keyremap:dbg] target_pid={} app={} key=0x{:02X}({}) mods={:?} -> {:?}",
            target_pid,
            bundle_id,
            keycode,
            keycode::key_name(keycode),
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
            tag_remapped_key_event(event, mods, keycode);
            let new_flags = build_flags(flags, mods, new_mods);
            event.set_flags(new_flags);
            event.set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, key as i64);
            CallbackResult::Keep
        }
        KeyAction::Char { ref text } => {
            tag_remapped_key_event(event, mods, keycode);
            let clean_flags = strip_all_modifiers(flags);
            event.set_flags(clean_flags);
            // Set keycode to SPACE so dead-key positions (like ´ on Nordic)
            // don't trigger the input method's dead-key state machine.
            event
                .set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, keycode::SPACE as i64);
            event.set_string(text);
            CallbackResult::Keep
        }
    }
}

fn tag_remapped_key_event(
    event: &core_graphics::event::CGEvent,
    original_mods: Modifiers,
    original_key: u16,
) {
    event.set_integer_value_field(
        EventField::EVENT_SOURCE_USER_DATA,
        keyremap_marker::encode(marker_mod_bits(original_mods), original_key),
    );
}

fn marker_mod_bits(mods: Modifiers) -> u8 {
    let mut bits = 0;
    if mods.ctrl {
        bits |= keyremap_marker::MOD_CTRL;
    }
    if mods.shift {
        bits |= keyremap_marker::MOD_SHIFT;
    }
    if mods.alt {
        bits |= keyremap_marker::MOD_ALT;
    }
    if mods.cmd {
        bits |= keyremap_marker::MOD_SUPER;
    }
    bits
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
