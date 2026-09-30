use std::mem::ManuallyDrop;
use std::os::raw::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use core_foundation::base::TCFType;
use core_foundation::mach_port::{CFMachPort, CFMachPortInvalidate, CFMachPortRef};
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType,
    CallbackResult, EventField,
};
use core_graphics::sys::CGEventRef;
use foreign_types_shared::ForeignType;

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

use super::app::remap::{self, MouseAction, MouseButton, ResolvedConfig, ScrollAction};
use super::app_tracker::AppTracker;
use super::input::backends::event_tap::{self, build_flags, extract_modifiers};
use super::input::{InputState, Strategy};

pub struct TapState {
    config: RwLock<Arc<ResolvedConfig>>,
    app_tracker: Arc<AppTracker>,
    input: Arc<InputState>,
}

impl TapState {
    pub fn new(
        config: ResolvedConfig,
        app_tracker: Arc<AppTracker>,
        input: Arc<InputState>,
    ) -> Self {
        Self {
            config: RwLock::new(Arc::new(config)),
            app_tracker,
            input,
        }
    }

    pub fn swap_config(&self, new_config: ResolvedConfig) {
        let new = Arc::new(new_config);
        if let Ok(mut guard) = self.config.write() {
            *guard = new;
        }
    }

    pub(crate) fn config(&self) -> Arc<ResolvedConfig> {
        self.config
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    pub(crate) fn input(&self) -> &Arc<InputState> {
        &self.input
    }

    pub(crate) fn key_target_bundle_id(&self) -> String {
        self.app_tracker
            .bundle_id_for_next_key(!super::secure_input::enabled())
    }
}

impl super::input::backends::virtual_hid::KeyEnvironment for TapState {
    fn config(&self) -> Arc<ResolvedConfig> {
        TapState::config(self)
    }

    fn key_target_bundle_id(&self) -> String {
        TapState::key_target_bundle_id(self)
    }

    fn input(&self) -> &Arc<InputState> {
        TapState::input(self)
    }
}

pub fn start_tap(state: Arc<TapState>) {
    std::thread::Builder::new()
        .name("keyremap-tap".into())
        .spawn(move || run_tap(state))
        .expect("failed to spawn tap thread");
}

fn wait_for_accessibility() {
    if request_accessibility_trust() {
        return;
    }

    log::warn!("waiting for Accessibility permission...");
    log::warn!("grant in System Settings > Privacy & Security > Accessibility");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if accessibility_trusted() {
            log::info!("Accessibility permission granted");
            return;
        }
    }
}

pub(super) fn accessibility_trusted() -> bool {
    qol_platform::permission_status(qol_platform::Permission::InputCapture).is_allowed()
}

fn request_accessibility_trust() -> bool {
    qol_platform::request_permission(qol_platform::Permission::InputCapture).is_allowed()
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
            log::error!("failed to create event tap (even with Accessibility granted)");
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
            log::warn!("panic in event callback - passing event through");
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
    let target_pid =
        i32::try_from(event.get_integer_value_field(EventField::EVENT_TARGET_UNIX_PROCESS_ID))
            .unwrap_or_default();
    if matches!(
        event_type,
        CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged
    ) {
        state.app_tracker.note_key_target(target_pid);
    }
    if matches!(event_type, CGEventType::FlagsChanged) {
        return CallbackResult::Keep;
    }

    let config = state.config();
    if !config.enabled {
        return CallbackResult::Keep;
    }
    let bundle_id = state.app_tracker.bundle_id_for_target(target_pid);

    match event_type {
        CGEventType::KeyDown | CGEventType::KeyUp => match state.input.strategy.get() {
            Strategy::EventTap => {
                event_tap::handle_key_event(config.as_ref(), event, target_pid, &bundle_id)
            }
            Strategy::VirtualHid => event_tap::stamp_marker(
                &state.input.markers,
                event,
                matches!(event_type, CGEventType::KeyDown),
            ),
        },
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
