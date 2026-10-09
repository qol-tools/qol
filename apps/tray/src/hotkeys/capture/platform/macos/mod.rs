use super::super::binding::{Binding, CaptureEvent};
use super::super::{OnFire, RebuildBindings};
use super::key_matcher::{self, KeyCombo, KeyMatcher, KeyTransition};
use anyhow::{bail, Result};
use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use crossbeam_channel::Receiver;
use layout::LayoutSymbols;
use qol_hotkeys::grammar::{Key, Modifier as Mod};
use qol_hotkeys::macos_keycode;
use qol_runtime::event_tap_trace::{TraceSink, QUEUE_DEPTH};
use qol_runtime::keyremap_marker::{self, KeyRemapMarker};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

mod layout;
mod recorder;

pub(crate) fn start_recording(session_id: u64, events: Arc<crate::daemon::EventBus>) -> bool {
    recorder::global().start(session_id, events)
}

pub(crate) fn cancel_recording(session_id: u64) {
    recorder::global().cancel(session_id);
}

/// The macOS tap passes events through instead of re-emitting them, so it
/// holds no synthetic key state to flush.
pub(crate) fn release_held_keys() {}

pub(crate) fn install(
    bindings: Vec<Binding>,
    on_fire: OnFire,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) -> Result<()> {
    let matcher = Arc::new(RwLock::new(KeyMatcher::new(bindings, parse_mac_combo)));
    let fire_tx = key_matcher::spawn_fire_thread(on_fire)?;
    key_matcher::spawn_reload_thread(matcher.clone(), reload_rx, rebuild, fire_tx.clone());

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let tap_matcher = matcher.clone();
    std::thread::Builder::new()
        .name("hotkey-capture-macos".into())
        .spawn(move || run_tap(tap_matcher, fire_tx, ready_tx))?;

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => bail!(message),
        Err(RecvTimeoutError::Timeout) => {
            bail!("macOS hotkey event tap did not report readiness")
        }
        Err(RecvTimeoutError::Disconnected) => {
            bail!("macOS hotkey event tap exited before reporting readiness")
        }
    }
}

fn requires_reenable(event_type: CGEventType) -> bool {
    matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    )
}

struct ReenablePort(CFMachPortRef);

unsafe impl Send for ReenablePort {}
unsafe impl Sync for ReenablePort {}

extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

fn tap_trace() -> &'static TraceSink {
    static SINK: OnceLock<TraceSink> = OnceLock::new();
    SINK.get_or_init(|| {
        TraceSink::spawn("hotkey-tap-trace", QUEUE_DEPTH, |batch| {
            for line in batch {
                log::warn!("{line}");
            }
        })
    })
}

static TAP_PORT: OnceLock<ReenablePort> = OnceLock::new();
static TAP_RELEASED: AtomicBool = AtomicBool::new(false);

pub(crate) fn release_tap() {
    if TAP_RELEASED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(port) = TAP_PORT.get() {
        unsafe { CGEventTapEnable(port.0, false) };
    }
}

fn reenable_target() -> Option<CFMachPortRef> {
    reenable_gate(TAP_RELEASED.load(Ordering::SeqCst), TAP_PORT.get())
}

fn reenable_gate(released: bool, port: Option<&ReenablePort>) -> Option<CFMachPortRef> {
    if released {
        return None;
    }
    port.map(|port| port.0)
}

/// A HID tap is created successfully without Accessibility permission and then
/// receives no events, so the tap alone can never tell us we are untrusted.
fn accessibility_trusted() -> bool {
    qol_platform::request_permission(qol_platform::Permission::InputCapture).is_allowed()
}

fn run_tap(
    matcher: Arc<RwLock<KeyMatcher>>,
    fire_tx: Sender<CaptureEvent>,
    ready_tx: Sender<Result<(), String>>,
) {
    // Only error and startup levels reach the installed tray's log file, so a
    // warning here would never be readable when it matters.
    if !accessibility_trusted() {
        log::error!(
            "macOS Accessibility permission is not granted to qol-tray; the hotkey tap will receive no events"
        );
    } else {
        log::error!("macOS Accessibility permission is granted to qol-tray");
    }
    let events = vec![CGEventType::KeyDown, CGEventType::KeyUp];
    let armed_matcher = Arc::clone(&matcher);
    let tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::TailAppendEventTap,
        CGEventTapOptions::Default,
        events,
        move |_proxy, event_type, event| {
            if requires_reenable(event_type) {
                if let Some(port) = reenable_target() {
                    tap_trace().offer("macOS hotkey tap disabled by OS; re-enabling".to_owned());
                    unsafe { CGEventTapEnable(port, true) };
                }
                return CallbackResult::Keep;
            }
            if !matches!(event_type, CGEventType::KeyDown | CGEventType::KeyUp) {
                return CallbackResult::Keep;
            }
            let recorder = recorder::global();
            if recorder.recording() {
                if matches!(event_type, CGEventType::KeyDown) && !is_auto_repeat(event) {
                    recorder.handle_event(event_type, &observed_combo(event));
                }
                return CallbackResult::Drop;
            }
            let transition = match event_type {
                CGEventType::KeyUp => KeyTransition::Release,
                _ if is_auto_repeat(event) => KeyTransition::Repeat,
                _ => KeyTransition::Press,
            };
            static SEEN: AtomicBool = AtomicBool::new(false);
            if !SEEN.swap(true, Ordering::Relaxed) {
                log::error!("macOS hotkey tap received its first key event");
            }
            let observed = observed_combo(event);
            let outcome = key_matcher::match_under_lock(&matcher, transition, &observed);
            if let Some((binding, phase)) = outcome.fired {
                let _ = fire_tx.send(CaptureEvent { binding, phase });
            }
            if outcome.swallow {
                CallbackResult::Drop
            } else {
                CallbackResult::Keep
            }
        },
    );

    let tap = match tap {
        Ok(tap) => tap,
        Err(()) => {
            let _ = ready_tx.send(Err(
                "failed to create macOS hotkey event tap; grant qol-tray Accessibility permission"
                    .into(),
            ));
            return;
        }
    };
    let _ = TAP_PORT.set(ReenablePort(tap.mach_port().as_concrete_TypeRef()));

    let loop_source = match tap.mach_port().create_runloop_source(0) {
        Ok(loop_source) => loop_source,
        Err(()) => {
            let _ = ready_tx.send(Err(
                "failed to create run loop source for macOS hotkey event tap".into(),
            ));
            return;
        }
    };
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    log::error!(
        "macOS hotkey tap armed with {} bindings",
        armed_matcher.read().map(|m| m.binding_count()).unwrap_or(0)
    );
    let _ = ready_tx.send(Ok(()));
    CFRunLoop::run_current();
}

fn observed_combo(event: &CGEvent) -> KeyCombo {
    let marker = remap_marker(event);
    KeyCombo {
        mods: marker
            .map(|marker| marker_mods(marker.mods))
            .unwrap_or_else(|| event_mods(event.get_flags())),
        key: marker
            .map(|marker| marker.key)
            .unwrap_or_else(|| event_key(event)),
    }
}

fn remap_marker(event: &CGEvent) -> Option<KeyRemapMarker> {
    keyremap_marker::decode(event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA))
}

fn event_key(event: &CGEvent) -> u16 {
    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16
}

fn is_auto_repeat(event: &CGEvent) -> bool {
    event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0
}

fn event_mods(flags: CGEventFlags) -> BTreeSet<Mod> {
    let mut mods = BTreeSet::new();
    if flags.contains(CGEventFlags::CGEventFlagShift) {
        mods.insert(Mod::Shift);
    }
    if flags.contains(CGEventFlags::CGEventFlagControl) {
        mods.insert(Mod::Ctrl);
    }
    if flags.contains(CGEventFlags::CGEventFlagAlternate) {
        mods.insert(Mod::Alt);
    }
    if flags.contains(CGEventFlags::CGEventFlagCommand) {
        mods.insert(Mod::Super);
    }
    mods
}

fn marker_mods(bits: u8) -> BTreeSet<Mod> {
    let mut mods = BTreeSet::new();
    if bits & keyremap_marker::MOD_SHIFT != 0 {
        mods.insert(Mod::Shift);
    }
    if bits & keyremap_marker::MOD_CTRL != 0 {
        mods.insert(Mod::Ctrl);
    }
    if bits & keyremap_marker::MOD_ALT != 0 {
        mods.insert(Mod::Alt);
    }
    if bits & keyremap_marker::MOD_SUPER != 0 {
        mods.insert(Mod::Super);
    }
    mods
}

fn parse_mac_combo(binding: &Binding) -> Option<KeyCombo> {
    let combo = binding.combo.as_ref()?;
    let key = match combo.key {
        Key::Symbol(symbol) => LayoutSymbols::current().keycode_of(symbol)?,
        key => macos_keycode::key_to_keycode(key)?,
    };
    Some(KeyCombo {
        mods: combo.mods.clone(),
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;

    #[test]
    fn a_released_tap_is_never_re_armed_by_a_late_disable_event() {
        let port = ReenablePort(1 as CFMachPortRef);
        assert!(
            reenable_gate(false, Some(&port)).is_some(),
            "a live tray re-enables its tap so hotkeys survive an OS timeout"
        );
        assert!(
            reenable_gate(true, Some(&port)).is_none(),
            "a tray that has begun shutting down must never re-arm its HID tap, or every keystroke stays gated until it exits"
        );
    }

    fn binding(key: &str) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_uid: crate::plugins::PluginUid::new("plugin"),
            action: "open".into(),
            raw_key: key.into(),
            continuous: false,
        }
    }

    #[test]
    fn os_disabled_tap_events_require_reenable_others_do_not() {
        let cases = [
            (CGEventType::TapDisabledByTimeout, true),
            (CGEventType::TapDisabledByUserInput, true),
            (CGEventType::KeyDown, false),
            (CGEventType::FlagsChanged, false),
            (CGEventType::Null, false),
        ];
        for (event_type, expected) in cases {
            assert_eq!(
                requires_reenable(event_type),
                expected,
                "event_type: {event_type:?}"
            );
        }
    }

    #[test]
    fn parses_shift_super_r_as_macos_keycode() {
        let combo = parse_mac_combo(&binding("Shift+Super+R")).expect("combo");
        assert_eq!(combo.key, 15);
        assert_eq!(combo.mods, BTreeSet::from([Mod::Shift, Mod::Super]));
    }

    #[test]
    fn a_symbol_binding_resolves_to_the_key_that_types_it() {
        for (key, symbol) in [("Super+Plus", '+'), ("Super+-", '-')] {
            let combo = parse_mac_combo(&binding(key)).expect("symbol bindings must fire");
            assert_eq!(
                Some(combo.key),
                LayoutSymbols::current().keycode_of(symbol),
                "key: {key}"
            );
        }
    }

    #[test]
    fn marker_mods_round_trip_to_combo_mods() {
        let bits = keyremap_marker::MOD_CTRL | keyremap_marker::MOD_SUPER;
        assert_eq!(marker_mods(bits), BTreeSet::from([Mod::Ctrl, Mod::Super]));
    }

    #[test]
    fn original_ctrl_does_not_match_synthetic_super_binding() {
        let observed = KeyCombo {
            mods: BTreeSet::from([Mod::Ctrl]),
            key: 15,
        };

        let super_matcher = KeyMatcher::new(vec![binding("Super+R")], parse_mac_combo);
        assert!(super_matcher.match_combo(&observed).is_none());

        let ctrl_matcher = KeyMatcher::new(vec![binding("Ctrl+R")], parse_mac_combo);
        assert!(ctrl_matcher.match_combo(&observed).is_some());
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_mac_combo(&binding("Super+Nope")).is_none());
    }
}
