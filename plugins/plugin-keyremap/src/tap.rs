use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};

use crate::app_tracker::AppTracker;
use crate::remap::{self, KeyAction, Modifiers, MouseAction, MouseButton, ResolvedConfig, ScrollAction};

pub struct TapState {
    config: AtomicPtr<ResolvedConfig>,
    app_tracker: Arc<AppTracker>,
}

impl TapState {
    pub fn new(config: ResolvedConfig, app_tracker: Arc<AppTracker>) -> Self {
        let config = Arc::new(config);
        Self {
            config: AtomicPtr::new(Arc::into_raw(config) as *mut ResolvedConfig),
            app_tracker,
        }
    }

    pub fn swap_config(&self, new_config: ResolvedConfig) {
        let new_ptr = Arc::into_raw(Arc::new(new_config)) as *mut ResolvedConfig;
        let old_ptr = self.config.swap(new_ptr, Ordering::AcqRel);
        if !old_ptr.is_null() {
            unsafe {
                drop(Arc::from_raw(old_ptr));
            }
        }
    }

    fn config(&self) -> Arc<ResolvedConfig> {
        let ptr = self.config.load(Ordering::Acquire);
        assert!(!ptr.is_null(), "TapState config pointer should never be null");
        unsafe {
            Arc::increment_strong_count(ptr);
            Arc::from_raw(ptr)
        }
    }
}

impl Drop for TapState {
    fn drop(&mut self) {
        let ptr = *self.config.get_mut();
        if !ptr.is_null() {
            unsafe {
                drop(Arc::from_raw(ptr));
            }
        }
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

    let result = CGEventTap::with_enabled(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        events,
        move |_proxy, event_type, event| handle_event(&state, event_type, event),
        || CFRunLoop::run_current(),
    );

    if result.is_err() {
        eprintln!("[keyremap] failed to create event tap (even with Accessibility granted)");
        std::process::exit(1);
    }
}

fn handle_event(
    state: &TapState,
    event_type: CGEventType,
    event: &core_graphics::event::CGEvent,
) -> CallbackResult {
    if matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    ) {
        return CallbackResult::Keep;
    }

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
        CGEventType::ScrollWheel => {
            handle_scroll_event(config.as_ref(), event, &bundle_id)
        }
        _ => CallbackResult::Keep,
    }
}

fn handle_key_event(
    config: &ResolvedConfig,
    event: &core_graphics::event::CGEvent,
    bundle_id: &str,
) -> CallbackResult {
    let flags = event.get_flags();
    let mods = extract_modifiers(flags);
    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;

    match remap::process_key_event(config, mods, keycode, bundle_id) {
        KeyAction::Passthrough => CallbackResult::Keep,
        KeyAction::Remap { mods: new_mods, key } => {
            let new_flags = build_flags(flags, mods, new_mods);
            event.set_flags(new_flags);
            event.set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, key as i64);
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

fn extract_modifiers(flags: CGEventFlags) -> Modifiers {
    Modifiers {
        ctrl: flags.contains(CGEventFlags::CGEventFlagControl),
        shift: flags.contains(CGEventFlags::CGEventFlagShift),
        alt: flags.contains(CGEventFlags::CGEventFlagAlternate),
        cmd: flags.contains(CGEventFlags::CGEventFlagCommand),
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
