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

unsafe impl Send for TapState {}
unsafe impl Sync for TapState {}

impl TapState {
    pub fn new(config: ResolvedConfig, app_tracker: Arc<AppTracker>) -> Self {
        Self {
            config: AtomicPtr::new(Box::into_raw(Box::new(config))),
            app_tracker,
        }
    }

    pub fn swap_config(&self, new_config: ResolvedConfig) {
        let new_ptr = Box::into_raw(Box::new(new_config));
        let old_ptr = self.config.swap(new_ptr, Ordering::AcqRel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            drop(unsafe { Box::from_raw(old_ptr) });
        });
    }

    fn config(&self) -> &ResolvedConfig {
        unsafe { &*self.config.load(Ordering::Acquire) }
    }
}

impl Drop for TapState {
    fn drop(&mut self) {
        let ptr = *self.config.get_mut();
        if !ptr.is_null() {
            drop(unsafe { Box::from_raw(ptr) });
        }
    }
}

pub fn start_tap(state: Arc<TapState>) {
    std::thread::Builder::new()
        .name("keyremap-tap".into())
        .spawn(move || run_tap(state))
        .expect("failed to spawn tap thread");
}

fn run_tap(state: Arc<TapState>) {
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
        eprintln!("[keyremap] failed to create event tap");
        eprintln!("[keyremap] grant Accessibility permission in System Settings > Privacy & Security > Accessibility");
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
    let bundle_id = state.app_tracker.bundle_id();

    match event_type {
        CGEventType::KeyDown | CGEventType::KeyUp => {
            handle_key_event(config, event, &bundle_id)
        }
        CGEventType::LeftMouseDown | CGEventType::LeftMouseUp => {
            handle_mouse_event(config, event, MouseButton::Left, &bundle_id)
        }
        CGEventType::RightMouseDown | CGEventType::RightMouseUp => {
            handle_mouse_event(config, event, MouseButton::Right, &bundle_id)
        }
        CGEventType::ScrollWheel => {
            handle_scroll_event(config, event, &bundle_id)
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
